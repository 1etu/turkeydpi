use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use engine::{BypassConfig, BypassEngine, DetectedProtocol, DohResolver, Pipeline, Stats};

use crate::error::{BackendError, Result};
use crate::traits::{Backend, BackendConfig, BackendHandle, BackendSettings, ProxySettings};

const SOCKS5_VERSION: u8 = 0x05;
const SOCKS5_NO_AUTH: u8 = 0x00;
const SOCKS5_CMD_CONNECT: u8 = 0x01;
const SOCKS5_ATYP_IPV4: u8 = 0x01;
const SOCKS5_ATYP_DOMAIN: u8 = 0x03;
const SOCKS5_ATYP_IPV6: u8 = 0x04;

const REPLY_SUCCESS: u8 = 0x00;
const REPLY_HOST_UNREACHABLE: u8 = 0x04;
const REPLY_CMD_NOT_SUPPORTED: u8 = 0x07;
const REPLY_ATYP_NOT_SUPPORTED: u8 = 0x08;

pub struct ProxyBackend {
    running: Arc<AtomicBool>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    config: Option<ProxySettings>,
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    active_connections: Arc<AtomicU64>,
}

impl ProxyBackend {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            config: None,
            task_handle: Mutex::new(None),
            active_connections: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn handle_socks5(
        mut client: TcpStream,
        client_addr: SocketAddr,
        bypass: BypassConfig,
        dns: Arc<DohResolver>,
        active_conns: Arc<AtomicU64>,
    ) {
        let _guard = ConnectionGuard::new(active_conns);

        debug!(client = %client_addr, "New SOCKS5 connection");

        let target = match Self::socks5_handshake(&mut client).await {
            Ok(Some(target)) => target,
            Ok(None) => return,
            Err(e) => {
                debug!(client = %client_addr, error = %e, "SOCKS5 handshake failed");
                return;
            }
        };

        let addrs = match dns.resolve_socket_addrs(&target).await {
            Ok(addrs) => addrs,
            Err(e) => {
                warn!(target = %target, error = %e, "SOCKS5 resolution failed");
                let _ = Self::send_reply(&mut client, REPLY_HOST_UNREACHABLE).await;
                return;
            }
        };

        let mut connected = None;
        for addr in addrs {
            if let Ok(stream) = TcpStream::connect(addr).await {
                connected = Some(stream);
                break;
            }
        }

        let mut remote = match connected {
            Some(stream) => stream,
            None => {
                warn!(target = %target, "SOCKS5 connect failed");
                let _ = Self::send_reply(&mut client, REPLY_HOST_UNREACHABLE).await;
                return;
            }
        };

        if Self::send_reply(&mut client, REPLY_SUCCESS).await.is_err() {
            return;
        }

        let _ = client.set_nodelay(true);
        let _ = remote.set_nodelay(true);

        if Self::forward_first_write(&mut client, &mut remote, &bypass)
            .await
            .is_err()
        {
            return;
        }

        let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
    }

    async fn socks5_handshake(client: &mut TcpStream) -> std::io::Result<Option<String>> {
        let mut greeting = [0u8; 2];
        client.read_exact(&mut greeting).await?;

        if greeting[0] != SOCKS5_VERSION {
            return Ok(None);
        }

        let mut methods = vec![0u8; greeting[1] as usize];
        client.read_exact(&mut methods).await?;

        if !methods.contains(&SOCKS5_NO_AUTH) {
            client.write_all(&[SOCKS5_VERSION, 0xFF]).await?;
            return Ok(None);
        }

        client.write_all(&[SOCKS5_VERSION, SOCKS5_NO_AUTH]).await?;

        let mut request = [0u8; 4];
        client.read_exact(&mut request).await?;

        if request[1] != SOCKS5_CMD_CONNECT {
            Self::send_reply(client, REPLY_CMD_NOT_SUPPORTED).await?;
            return Ok(None);
        }

        let host = match request[3] {
            SOCKS5_ATYP_IPV4 => {
                let mut addr = [0u8; 4];
                client.read_exact(&mut addr).await?;
                std::net::Ipv4Addr::from(addr).to_string()
            }
            SOCKS5_ATYP_IPV6 => {
                let mut addr = [0u8; 16];
                client.read_exact(&mut addr).await?;
                format!("[{}]", std::net::Ipv6Addr::from(addr))
            }
            SOCKS5_ATYP_DOMAIN => {
                let mut len = [0u8; 1];
                client.read_exact(&mut len).await?;
                let mut domain = vec![0u8; len[0] as usize];
                client.read_exact(&mut domain).await?;
                String::from_utf8(domain).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid domain")
                })?
            }
            _ => {
                Self::send_reply(client, REPLY_ATYP_NOT_SUPPORTED).await?;
                return Ok(None);
            }
        };

        let mut port_buf = [0u8; 2];
        client.read_exact(&mut port_buf).await?;
        let port = u16::from_be_bytes(port_buf);

        Ok(Some(format!("{}:{}", host, port)))
    }

    async fn send_reply(client: &mut TcpStream, code: u8) -> std::io::Result<()> {
        let reply = [
            SOCKS5_VERSION,
            code,
            0x00,
            SOCKS5_ATYP_IPV4,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        client.write_all(&reply).await
    }

    async fn forward_first_write(
        client: &mut TcpStream,
        remote: &mut TcpStream,
        bypass: &BypassConfig,
    ) -> std::io::Result<()> {
        let mut buf = vec![0u8; 65536];

        let n = client.read(&mut buf).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed",
            ));
        }

        let engine = BypassEngine::new(bypass.clone());
        let result = engine.process_outgoing(&buf[..n]);

        if let Some(ref host) = result.hostname {
            match result.protocol {
                DetectedProtocol::TlsClientHello if result.modified => {
                    info!("{} [SNI fragmented]", host)
                }
                DetectedProtocol::HttpRequest if result.modified => {
                    info!("{} [Host fragmented]", host)
                }
                _ => debug!("{} [passthrough]", host),
            }
        }

        let last = result.fragments.len().saturating_sub(1);
        for (i, fragment) in result.fragments.iter().enumerate() {
            remote.write_all(fragment).await?;
            if i < last {
                if let Some(delay) = result.inter_fragment_delay {
                    tokio::time::sleep(delay).await;
                }
            }
        }

        remote.flush().await
    }
}

struct ConnectionGuard {
    counter: Arc<AtomicU64>,
}

impl ConnectionGuard {
    fn new(counter: Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for ProxyBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ProxyBackend {
    fn name(&self) -> &'static str {
        "proxy"
    }

    async fn start(&mut self, config: BackendConfig) -> Result<BackendHandle> {
        if self.running.load(Ordering::SeqCst) {
            return Err(BackendError::AlreadyRunning);
        }

        let BackendSettings::Proxy(proxy_settings) = config.backend_settings;

        info!(addr = %proxy_settings.listen_addr, "Starting SOCKS5 backend");

        let listener = TcpListener::bind(proxy_settings.listen_addr)
            .await
            .map_err(|e| BackendError::BindFailed(e.to_string()))?;

        let stats = Arc::new(Stats::new());
        let pipeline = Arc::new(
            Pipeline::new(config.engine_config, stats.clone()).map_err(BackendError::Engine)?,
        );

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        self.config = Some(proxy_settings.clone());
        self.shutdown_tx = Some(shutdown_tx.clone());
        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let max_connections = proxy_settings.max_connections;
        let active_connections = self.active_connections.clone();
        let bypass = proxy_settings.bypass.clone();
        let dns = Arc::new(DohResolver::new());

        let handle = tokio::spawn(async move {
            info!("SOCKS5 backend accepting connections");

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("SOCKS5 backend received shutdown signal");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                if active_connections.load(Ordering::Relaxed) >= max_connections as u64 {
                                    warn!(addr = %addr, "Connection limit reached, rejecting");
                                    continue;
                                }

                                tokio::spawn(Self::handle_socks5(
                                    stream,
                                    addr,
                                    bypass.clone(),
                                    dns.clone(),
                                    active_connections.clone(),
                                ));
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to accept connection");
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("SOCKS5 backend stopped");
        });

        *self.task_handle.lock() = Some(handle);

        Ok(BackendHandle {
            shutdown_tx,
            stats,
            pipeline,
        })
    }

    async fn stop(&mut self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(BackendError::NotRunning);
        }

        info!("Stopping SOCKS5 backend");

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        let handle = self.task_handle.lock().take();
        if let Some(handle) = handle {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        self.running.store(false, Ordering::SeqCst);
        self.config = None;

        info!("SOCKS5 backend stopped");
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn is_supported() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Config;

    #[test]
    fn test_backend_creation() {
        let backend = ProxyBackend::new();
        assert!(!backend.is_running());
    }

    #[test]
    fn test_proxy_supported() {
        assert!(ProxyBackend::is_supported());
    }

    #[tokio::test]
    async fn test_backend_start_stop() {
        let mut backend = ProxyBackend::new();

        let config = BackendConfig {
            engine_config: Config::default(),
            max_queue_size: 100,
            backend_settings: BackendSettings::Proxy(ProxySettings {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
        };

        let handle = backend.start(config).await.unwrap();
        assert!(backend.is_running());
        drop(handle);

        backend.stop().await.unwrap();
        assert!(!backend.is_running());
    }

    #[test]
    fn test_connection_guard() {
        let counter = Arc::new(AtomicU64::new(0));

        {
            let _guard = ConnectionGuard::new(counter.clone());
            assert_eq!(counter.load(Ordering::Relaxed), 1);
        }

        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
