use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::BytesMut;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use engine::{FlowKey, Pipeline, Stats};
use engine::config::Protocol;

use crate::error::{BackendError, Result};
use crate::traits::{Backend, BackendConfig, BackendHandle, BackendSettings, ProxySettings, ProxyType};

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
        pipeline: Arc<Pipeline>,
        stats: Arc<Stats>,
        active_conns: Arc<AtomicU64>,
    ) {
        let _guard = ConnectionGuard::new(active_conns);
        
        debug!(client = %client_addr, "New SOCKS5 connection");
        
        let mut buf = [0u8; 2];
        if client.read_exact(&mut buf).await.is_err() {
            return;
        }
        
        let version = buf[0];
        let nmethods = buf[1] as usize;
        
        if version != 0x05 {
            warn!(version, "inv SOCKS version");
            return;
        }
        
        let mut methods = vec![0u8; nmethods];
        if client.read_exact(&mut methods).await.is_err() {
            return;
        }
        
        if !methods.contains(&0x00) {
            let _ = client.write_all(&[0x05, 0xFF]).await;
            return;
        }
        
        if client.write_all(&[0x05, 0x00]).await.is_err() {
            return;
        }
        
        let mut request = [0u8; 4];
        if client.read_exact(&mut request).await.is_err() {
            return;
        }
        
        let cmd = request[1];
        let atyp = request[3];
        
        if cmd != 0x01 {
            let response = [0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
            let _ = client.write_all(&response).await;
            return;
        }
        
        let (dst_addr, dst_port) = match atyp {
            0x01 => {
                let mut addr = [0u8; 4];
                if client.read_exact(&mut addr).await.is_err() {
                    return;
                }
                let mut port_buf = [0u8; 2];
                if client.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                let port = u16::from_be_bytes(port_buf);
                let ip = std::net::Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]);
                (std::net::IpAddr::V4(ip), port)
            }
            0x03 => {
                let mut len = [0u8; 1];
                if client.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut domain = vec![0u8; len[0] as usize];
                if client.read_exact(&mut domain).await.is_err() {
                    return;
                }
                let mut port_buf = [0u8; 2];
                if client.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                let port = u16::from_be_bytes(port_buf);
                
                let domain_str = match String::from_utf8(domain) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                
                let resolved = match tokio::net::lookup_host(format!("{}:{}", domain_str, port)).await {
                    Ok(mut addrs) => match addrs.next() {
                        Some(addr) => addr,
                        None => return,
                    },
                    Err(_) => {
                        let response = [0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
                        let _ = client.write_all(&response).await;
                        return;
                    }
                };
                
                (resolved.ip(), port)
            }
            0x04 => {
                let mut addr = [0u8; 16];
                if client.read_exact(&mut addr).await.is_err() {
                    return;
                }
                let mut port_buf = [0u8; 2];
                if client.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                let port = u16::from_be_bytes(port_buf);
                let ip = std::net::Ipv6Addr::from(addr);
                (std::net::IpAddr::V6(ip), port)
            }
            _ => {
                let response = [0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
                let _ = client.write_all(&response).await;
                return;
            }
        };
        
        debug!(dst = %dst_addr, port = dst_port, "SOCKS5 CONNECT request");
        
        let remote = match TcpStream::connect((dst_addr, dst_port)).await {
            Ok(stream) => stream,
            Err(e) => {
                warn!(error = %e, dst = %dst_addr, port = dst_port, "Failed to connect");
                let response = [0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
                let _ = client.write_all(&response).await;
                return;
            }
        };
        
        let response = [0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        if client.write_all(&response).await.is_err() {
            return;
        }
        
        let flow_key = FlowKey::new(
            client_addr.ip(),
            dst_addr,
            client_addr.port(),
            dst_port,
            Protocol::Tcp,
        );
        
        Self::relay_streams(client, remote, flow_key, pipeline, stats).await;
    }

    async fn relay_streams(
        mut client: TcpStream,
        mut remote: TcpStream,
        flow_key: FlowKey,
        pipeline: Arc<Pipeline>,
        stats: Arc<Stats>,
    ) {
        let (mut client_read, mut client_write) = client.split();
        let (mut remote_read, mut remote_write) = remote.split();
        
        let _flow_key_rev = flow_key.reverse();
        let _pipeline_clone = pipeline.clone();
        let stats_clone = stats.clone();
        
        let outbound = async move {
            let mut buf = BytesMut::with_capacity(4096);
            buf.resize(4096, 0);
            
            loop {
                let n = match client_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                
                let data = BytesMut::from(&buf[..n]);
                
                match pipeline.process(flow_key, data) {
                    Ok(output) => {
                        for packet in output.all_packets() {
                            if remote_write.write_all(&packet).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Pipeline processing error");
                        break;
                    }
                }
            }
        };
        
        let inbound = async move {
            let mut buf = BytesMut::with_capacity(4096);
            buf.resize(4096, 0);
            
            loop {
                let n = match remote_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                
                if client_write.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                
                stats_clone.record_packet_in(n);
                stats_clone.record_packet_out(n);
            }
        };
        
        tokio::select! {
            _ = outbound => {}
            _ = inbound => {}
        }
        
        debug!(flow = ?flow_key, "Connection closed");
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

        let proxy_settings = match config.backend_settings {
            BackendSettings::Proxy(settings) => settings,
            _ => return Err(BackendError::NotSupported(
                "ProxyBackend requires ProxySettings".to_string()
            )),
        };

        info!(
            addr = %proxy_settings.listen_addr,
            proxy_type = ?proxy_settings.proxy_type,
            "Starting proxy backend"
        );

        let listener = TcpListener::bind(proxy_settings.listen_addr)
            .await
            .map_err(|e| BackendError::BindFailed(e.to_string()))?;

        let stats = Arc::new(Stats::new());
        let pipeline = Arc::new(
            Pipeline::new(config.engine_config, stats.clone())
                .map_err(|e| BackendError::Engine(e))?
        );

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        self.config = Some(proxy_settings.clone());
        self.shutdown_tx = Some(shutdown_tx.clone());
        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let pipeline_clone = pipeline.clone();
        let stats_clone = stats.clone();
        let max_connections = proxy_settings.max_connections;
        let active_connections = self.active_connections.clone();
        let proxy_type = proxy_settings.proxy_type;

        let handle = tokio::spawn(async move {
            info!("Proxy backend accepting connections");
            
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Proxy backend received shutdown signal");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                if active_connections.load(Ordering::Relaxed) >= max_connections as u64 {
                                    warn!(addr = %addr, "Connection limit reached, rejecting");
                                    continue;
                                }
                                
                                let pipeline = pipeline_clone.clone();
                                let stats = stats_clone.clone();
                                let active = active_connections.clone();
                                
                                match proxy_type {
                                    ProxyType::Socks5 => {
                                        tokio::spawn(Self::handle_socks5(
                                            stream, addr, pipeline, stats, active
                                        ));
                                    }
                                    ProxyType::HttpConnect => {
                                        warn!("--");
                                    }
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to accept connection");
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("Proxy backend stopped");
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

        info!("Stopping proxy backend");

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        let handle = self.task_handle.lock().take();
        if let Some(handle) = handle {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                handle,
            ).await;
        }

        self.running.store(false, Ordering::SeqCst);
        self.config = None;

        info!("Proxy backend stopped");
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
