use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::net::IpAddr;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, info};

use engine::{FlowKey, Pipeline, Stats};
use engine::config::Protocol;

use crate::error::{BackendError, Result};
use crate::traits::{Backend, BackendConfig, BackendHandle, BackendSettings, TunSettings};

pub struct TunBackend {
    running: Arc<AtomicBool>,    
    shutdown_tx: Option<mpsc::Sender<()>>,    
    config: Option<TunSettings>,    
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TunBackend {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            config: None,
            task_handle: Mutex::new(None),
        }
    }

    fn parse_ipv4_flow_key(data: &[u8]) -> Option<FlowKey> {
        if data.len() < 20 {
            return None;
        }

        let version = (data[0] >> 4) & 0x0F;
        if version != 4 {
            return None;
        }

        let ihl = (data[0] & 0x0F) as usize * 4;
        if data.len() < ihl {
            return None;
        }

        let protocol = data[9];
        let src_ip = IpAddr::V4(std::net::Ipv4Addr::new(
            data[12], data[13], data[14], data[15],
        ));
        let dst_ip = IpAddr::V4(std::net::Ipv4Addr::new(
            data[16], data[17], data[18], data[19],
        ));

        let (src_port, dst_port, proto) = match protocol {
            6 => {
                
                if data.len() < ihl + 4 {
                    return None;
                }
                let src_port = u16::from_be_bytes([data[ihl], data[ihl + 1]]);
                let dst_port = u16::from_be_bytes([data[ihl + 2], data[ihl + 3]]);
                (src_port, dst_port, Protocol::Tcp)
            }
            17 => {
                
                if data.len() < ihl + 4 {
                    return None;
                }
                let src_port = u16::from_be_bytes([data[ihl], data[ihl + 1]]);
                let dst_port = u16::from_be_bytes([data[ihl + 2], data[ihl + 3]]);
                (src_port, dst_port, Protocol::Udp)
            }
            1 => {
                
                (0, 0, Protocol::Icmp)
            }
            _ => {
                return None;
            }
        };

        Some(FlowKey::new(src_ip, dst_ip, src_port, dst_port, proto))
    }
}

impl Default for TunBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for TunBackend {
    fn name(&self) -> &'static str {
        "tun"
    }

    async fn start(&mut self, config: BackendConfig) -> Result<BackendHandle> {
        if self.running.load(Ordering::SeqCst) {
            return Err(BackendError::AlreadyRunning);
        }

        let tun_settings = match config.backend_settings {
            BackendSettings::Tun(settings) => settings,
            _ => return Err(BackendError::NotSupported(
                "TunBackend requires TunSettings".to_string()
            )),
        };

        info!(
            address = %tun_settings.address,
            mtu = tun_settings.mtu,
            "Starting TUN backend"
        );

        let stats = Arc::new(Stats::new());
        let pipeline = Arc::new(
            Pipeline::new(config.engine_config, stats.clone())
                .map_err(|e| BackendError::Engine(e))?
        );

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        self.config = Some(tun_settings.clone());
        self.shutdown_tx = Some(shutdown_tx.clone());
        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let pipeline_clone = pipeline.clone();
        let _stats_clone = stats.clone();

        let handle = tokio::spawn(async move {
            info!("TUN backend task started");
            let mut cleanup_interval = tokio::time::interval(
                std::time::Duration::from_secs(30)
            );
            
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("TUN backend received shutdown signal");
                        break;
                    }
                    _ = cleanup_interval.tick() => {
                        let evicted = pipeline_clone.cleanup();
                        if evicted > 0 {
                            debug!(evicted, "Cleaned up expired flows");
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("TUN backend task stopped");
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

        info!("Stopping TUN backend");

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

        info!("TUN backend stopped");
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn is_supported() -> bool {
        cfg!(target_os = "macos")
    }
}

#[cfg(test)]
pub struct MockTunDevice {
    read_queue: mpsc::Receiver<bytes::BytesMut>,
    write_queue: mpsc::Sender<bytes::BytesMut>,
}

#[cfg(test)]
impl MockTunDevice {
    pub fn new() -> (Self, mpsc::Sender<bytes::BytesMut>, mpsc::Receiver<bytes::BytesMut>) {
        let (read_tx, read_rx) = mpsc::channel(100);
        let (write_tx, write_rx) = mpsc::channel(100);
        
        (
            Self {
                read_queue: read_rx,
                write_queue: write_tx,
            },
            read_tx,
            write_rx,
        )
    }

    pub async fn read(&mut self) -> Option<bytes::BytesMut> {
        self.read_queue.recv().await
    }

    pub async fn write(&self, data: bytes::BytesMut) -> Result<()> {
        self.write_queue.send(data).await
            .map_err(|_| BackendError::QueueFull("write queue".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use engine::Config;

    fn create_ipv4_tcp_packet() -> BytesMut {
        let mut packet = BytesMut::with_capacity(60);
        
        
        packet.extend_from_slice(&[
            0x45, 0x00, 0x00, 0x3C, 
            0x00, 0x00, 0x00, 0x00, 
            0x40, 0x06, 0x00, 0x00, 
            192, 168, 1, 1,         
            8, 8, 8, 8,             
        ]);
        
        
        packet.extend_from_slice(&[
            0x30, 0x39,             
            0x01, 0xBB,             
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x50, 0x02,             
            0x00, 0x00,             
            0x00, 0x00,             
            0x00, 0x00,             
        ]);
        
        packet
    }

    #[test]
    fn test_parse_ipv4_flow_key() {
        let packet = create_ipv4_tcp_packet();
        let key = TunBackend::parse_ipv4_flow_key(&packet);
        
        assert!(key.is_some());
        let key = key.unwrap();
        
        assert_eq!(key.src_port, 12345);
        assert_eq!(key.dst_port, 443);
        assert!(matches!(key.protocol, Protocol::Tcp));
    }

    #[test]
    fn test_parse_invalid_packet() {
        let packet = BytesMut::from(&b"too short"[..]);
        let key = TunBackend::parse_ipv4_flow_key(&packet);
        assert!(key.is_none());
    }

    #[test]
    fn test_backend_creation() {
        let backend = TunBackend::new();
        assert!(!backend.is_running());
    }

    #[tokio::test]
    async fn test_backend_start_stop() {
        let mut backend = TunBackend::new();
        
        let config = BackendConfig {
            engine_config: Config::default(),
            max_queue_size: 100,
            backend_settings: BackendSettings::Tun(TunSettings::default()),
        };
        
        let handle = backend.start(config).await.unwrap();
        assert!(backend.is_running());
        
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        
        backend.stop().await.unwrap();
        assert!(!backend.is_running());
    }

    #[tokio::test]
    async fn test_backend_double_start() {
        let mut backend = TunBackend::new();
        
        let config = BackendConfig {
            engine_config: Config::default(),
            max_queue_size: 100,
            backend_settings: BackendSettings::Tun(TunSettings::default()),
        };
        
        let _handle = backend.start(config.clone()).await.unwrap();
        
        let result = backend.start(config).await;
        assert!(matches!(result, Err(BackendError::AlreadyRunning)));
        
        backend.stop().await.unwrap();
    }

    #[test]
    fn test_mock_device() {
        let (device, _read_tx, _write_rx) = MockTunDevice::new();
        drop(device);
    }
}
