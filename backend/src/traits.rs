use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::sync::mpsc;

use engine::{Config, FlowKey, Pipeline, Stats};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketDirection {
    Outbound,
    Inbound,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub data: BytesMut,
    pub direction: PacketDirection,    
    pub flow_key: Option<FlowKey>,    
    pub timestamp: std::time::Instant,
}

impl Packet {
    pub fn outbound(data: BytesMut) -> Self {
        Self {
            data,
            direction: PacketDirection::Outbound,
            flow_key: None,
            timestamp: std::time::Instant::now(),
        }
    }

    pub fn inbound(data: BytesMut) -> Self {
        Self {
            data,
            direction: PacketDirection::Inbound,
            flow_key: None,
            timestamp: std::time::Instant::now(),
        }
    }

    pub fn with_flow_key(mut self, key: FlowKey) -> Self {
        self.flow_key = Some(key);
        self
    }
}

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub engine_config: Config,    
    pub max_queue_size: usize,    
    pub backend_settings: BackendSettings,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            engine_config: Config::default(),
            max_queue_size: 1000,
            backend_settings: BackendSettings::Tun(TunSettings::default()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackendSettings {
    Tun(TunSettings),
    Proxy(ProxySettings),
}

#[derive(Debug, Clone)]
pub struct TunSettings {
    pub device_name: Option<String>,    
    pub mtu: u16,    
    pub address: String,    
    pub netmask: String,
}

impl Default for TunSettings {
    fn default() -> Self {
        Self {
            device_name: None,
            mtu: 1500,
            address: "10.0.85.1".to_string(),
            netmask: "255.255.255.0".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxySettings {
    pub listen_addr: SocketAddr,    
    pub proxy_type: ProxyType,    
    pub max_connections: usize,    
    pub timeout_secs: u64,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:1080".parse().unwrap(),
            proxy_type: ProxyType::Socks5,
            max_connections: 1000,
            timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyType {
    Socks5,
    HttpConnect,
}

pub struct BackendHandle {
    pub shutdown_tx: mpsc::Sender<()>,
    pub stats: Arc<Stats>,
    pub pipeline: Arc<Pipeline>,
}

impl BackendHandle {
    pub async fn shutdown(&self) -> Result<()> {
        self.shutdown_tx.send(()).await.map_err(|_| {
            crate::error::BackendError::NotRunning
        })?;
        Ok(())
    }

    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }

    pub fn reload_config(&self, config: Config) -> Result<()> {
        self.pipeline.reload_config(config)?;
        Ok(())
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn start(&mut self, config: BackendConfig) -> Result<BackendHandle>;
    async fn stop(&mut self) -> Result<()>;

    fn is_running(&self) -> bool;
    fn is_supported() -> bool

    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_creation() {
        let data = BytesMut::from(&b"test packet"[..]);
        let packet = Packet::outbound(data.clone());
        
        assert_eq!(packet.direction, PacketDirection::Outbound);
        assert!(packet.flow_key.is_none());
        assert_eq!(packet.data, data);
    }

    #[test]
    fn test_default_configs() {
        let tun = TunSettings::default();
        assert_eq!(tun.mtu, 1500);
        assert_eq!(tun.address, "10.0.85.1");
        
        let proxy = ProxySettings::default();
        assert_eq!(proxy.proxy_type, ProxyType::Socks5);
        assert_eq!(proxy.max_connections, 1000);
    }
}
