use bytes::{Bytes, BytesMut};
use std::time::Duration;

use crate::tls::{parse_client_hello, is_client_hello, is_http_request, find_http_host};

#[derive(Debug, Clone)]
pub struct BypassConfig {
    pub fragment_sni: bool,
    
    pub tls_split_pos: usize,
    
    pub fragment_http_host: bool,
    
    pub http_split_pos: usize,
    
    pub send_fake_packets: bool,
    
    pub fake_packet_ttl: u8,
    
    pub fragment_delay_us: u64,
    
    pub use_tcp_segmentation: bool,
    
    pub min_segment_size: usize,
    
    pub max_segment_size: usize,
}

impl Default for BypassConfig {
    fn default() -> Self {
        Self {
            fragment_sni: true,
            tls_split_pos: 3,  
            fragment_http_host: true,
            http_split_pos: 2, 
            send_fake_packets: false,
            fake_packet_ttl: 1,
            fragment_delay_us: 0,
            use_tcp_segmentation: true,
            min_segment_size: 1,
            max_segment_size: 40,
        }
    }
}

impl BypassConfig {
    pub fn turk_telekom() -> Self {
        Self {
            fragment_sni: true,
            tls_split_pos: 2,
            fragment_http_host: true,
            http_split_pos: 2,
            send_fake_packets: false,
            fake_packet_ttl: 1,
            fragment_delay_us: 0,
            use_tcp_segmentation: true,
            min_segment_size: 1,
            max_segment_size: 20,
        }
    }
    
    pub fn vodafone_tr() -> Self {
        Self {
            fragment_sni: true,
            tls_split_pos: 3,
            fragment_http_host: true,
            http_split_pos: 3,
            send_fake_packets: false,
            fake_packet_ttl: 1,
            fragment_delay_us: 100,
            use_tcp_segmentation: true,
            min_segment_size: 1,
            max_segment_size: 30,
        }
    }
    
    pub fn superonline() -> Self {
        Self {
            fragment_sni: true,
            tls_split_pos: 1,
            fragment_http_host: true,
            http_split_pos: 1,
            send_fake_packets: false,
            fake_packet_ttl: 1,
            fragment_delay_us: 0,
            use_tcp_segmentation: true,
            min_segment_size: 1,
            max_segment_size: 15,
        }
    }
    
    pub fn aggressive() -> Self {
        Self {
            fragment_sni: true,
            tls_split_pos: 0,  
            fragment_http_host: true,
            http_split_pos: 1,
            send_fake_packets: false,
            fake_packet_ttl: 3,
            fragment_delay_us: 10000,
            use_tcp_segmentation: true,
            min_segment_size: 1,
            max_segment_size: 5,
        }
    }
}

#[derive(Debug)]
pub struct BypassResult {
    pub fragments: Vec<Bytes>,    
    pub inter_fragment_delay: Option<Duration>,    
    pub fake_packet: Option<Bytes>,    
    pub modified: bool,
    pub protocol: DetectedProtocol,    
    pub hostname: Option<String>,
}

impl Default for BypassResult {
    fn default() -> Self {
        Self {
            fragments: Vec::new(),
            inter_fragment_delay: None,
            fake_packet: None,
            modified: false,
            protocol: DetectedProtocol::Unknown,
            hostname: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedProtocol {
    TlsClientHello,
    HttpRequest,
    Unknown,
}

pub struct BypassEngine {
    config: BypassConfig,
}

impl BypassEngine {
    pub fn new(config: BypassConfig) -> Self {
        Self { config }
    }

    pub fn process_outgoing(&self, data: &[u8]) -> BypassResult {
        let mut result = BypassResult::default();
        
        
        if is_client_hello(data) {
            result.protocol = DetectedProtocol::TlsClientHello;
            self.process_tls_client_hello(data, &mut result);
        } else if is_http_request(data) {
            result.protocol = DetectedProtocol::HttpRequest;
            self.process_http_request(data, &mut result);
        } else {
            
            result.fragments.push(Bytes::copy_from_slice(data));
        }
        
        result
    }
    
    fn process_tls_client_hello(&self, data: &[u8], result: &mut BypassResult) {
        if !self.config.fragment_sni {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }
        
        
        if let Some(info) = parse_client_hello(data) {
            result.hostname = info.sni_hostname.clone();
            
            
            
            let split_pos = if self.config.tls_split_pos > 0 {
                
                self.config.tls_split_pos.min(data.len() - 1)
            } else if let (Some(sni_off), Some(sni_len)) = (info.sni_offset, info.sni_length) {
                
                
                if sni_len > 2 {
                    sni_off + (sni_len / 2)
                } else {
                    sni_off
                }.min(data.len() - 1)
            } else {
                
                5.min(data.len() - 1)
            };
            
            
            if split_pos > 0 && split_pos < data.len() {
                
                let segment_size = self.config.max_segment_size.max(1);
                
                if segment_size < split_pos {
                    
                    let mut pos = 0;
                    while pos < split_pos {
                        let end = (pos + segment_size).min(split_pos);
                        result.fragments.push(Bytes::copy_from_slice(&data[pos..end]));
                        pos = end;
                    }
                    
                    result.fragments.push(Bytes::copy_from_slice(&data[split_pos..]));
                } else {
                    
                    result.fragments.push(Bytes::copy_from_slice(&data[..split_pos]));
                    result.fragments.push(Bytes::copy_from_slice(&data[split_pos..]));
                }
                result.modified = true;
                
                if self.config.fragment_delay_us > 0 {
                    result.inter_fragment_delay = Some(Duration::from_micros(self.config.fragment_delay_us));
                }
            } else {
                result.fragments.push(Bytes::copy_from_slice(data));
            }
        } else {
            
            result.fragments.push(Bytes::copy_from_slice(data));
        }
        
        
        if self.config.send_fake_packets && result.modified {
            result.fake_packet = Some(self.generate_fake_tls_packet(data));
        }
    }
    
    fn process_http_request(&self, data: &[u8], result: &mut BypassResult) {
        if !self.config.fragment_http_host {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }
        
        
        if let Some((host_offset, host_len)) = find_http_host(data) {
            result.hostname = std::str::from_utf8(&data[host_offset..host_offset + host_len])
                .ok()
                .map(|s| s.to_string());
            
            
            if let Some(host_header_pos) = find_host_header_start(data) {
                
                let split_pos = (host_header_pos + self.config.http_split_pos).min(data.len() - 1);
                
                if split_pos > 0 && split_pos < data.len() {
                    result.fragments.push(Bytes::copy_from_slice(&data[..split_pos]));
                    result.fragments.push(Bytes::copy_from_slice(&data[split_pos..]));
                    result.modified = true;
                    
                    if self.config.fragment_delay_us > 0 {
                        result.inter_fragment_delay = Some(Duration::from_micros(self.config.fragment_delay_us));
                    }
                } else {
                    result.fragments.push(Bytes::copy_from_slice(data));
                }
            } else {
                result.fragments.push(Bytes::copy_from_slice(data));
            }
        } else {
            result.fragments.push(Bytes::copy_from_slice(data));
        }
    }

    fn generate_fake_tls_packet(&self, original: &[u8]) -> Bytes {
        
        let mut fake = BytesMut::with_capacity(original.len());
        
        
        fake.extend_from_slice(original);
        
        
        if let Some(info) = parse_client_hello(original) {
            if let (Some(offset), Some(len)) = (info.sni_offset, info.sni_length) {
                if offset + len <= fake.len() {
                    
                    for i in 0..len {
                        fake[offset + i] = b'x';
                    }
                }
            }
        }
        
        fake.freeze()
    }
}

fn find_host_header_start(data: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(data).ok()?;
    let lower = text.to_lowercase();
    lower.find("\nhost:").map(|p| p + 1) 
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn sample_tls_client_hello() -> Vec<u8> {
        vec![
            0x16, 0x03, 0x01, 0x00, 0x5a,
            0x01, 0x00, 0x00, 0x56,
            0x03, 0x03,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
            0x00,
            0x00, 0x02, 0x13, 0x01,
            0x01, 0x00,
            0x00, 0x17,
            0x00, 0x00, 0x00, 0x10,
            0x00, 0x0e, 0x00, 0x00, 0x0b,
            0x64, 0x69, 0x73, 0x63, 0x6f, 0x72, 0x64, 0x2e, 0x63, 0x6f, 0x6d,
            0x00, 0x15, 0x00, 0x03, 0x00, 0x00, 0x00,
        ]
    }
    
    #[test]
    fn test_bypass_tls() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = sample_tls_client_hello();
        
        let result = engine.process_outgoing(&data);
        
        assert!(result.modified);
        assert_eq!(result.protocol, DetectedProtocol::TlsClientHello);
        assert!(result.fragments.len() >= 2);
        assert_eq!(result.hostname.as_deref(), Some("discord.com"));
        
        
        let mut reassembled = Vec::new();
        for frag in &result.fragments {
            reassembled.extend_from_slice(frag);
        }
        assert_eq!(reassembled, data);
    }
    
    #[test]
    fn test_bypass_http() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = b"GET / HTTP/1.1\r\nHost: discord.com\r\nConnection: close\r\n\r\n";
        
        let result = engine.process_outgoing(data);
        
        assert!(result.modified);
        assert_eq!(result.protocol, DetectedProtocol::HttpRequest);
        assert!(result.fragments.len() >= 2);
        assert_eq!(result.hostname.as_deref(), Some("discord.com"));
        
        
        let mut reassembled = Vec::new();
        for frag in &result.fragments {
            reassembled.extend_from_slice(frag);
        }
        assert_eq!(&reassembled[..], &data[..]);
    }
    
    #[test]
    fn test_isp_presets() {
        let data = sample_tls_client_hello();
        
        
        for config in [
            BypassConfig::turk_telekom(),
            BypassConfig::vodafone_tr(),
            BypassConfig::superonline(),
            BypassConfig::aggressive(),
        ] {
            let engine = BypassEngine::new(config);
            let result = engine.process_outgoing(&data);
            
            assert!(result.modified);
            
            
            let mut reassembled = Vec::new();
            for frag in &result.fragments {
                reassembled.extend_from_slice(frag);
            }
            assert_eq!(reassembled, data);
        }
    }
    
    #[test]
    fn test_unknown_protocol_passthrough() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = b"some random binary data\x00\x01\x02";
        
        let result = engine.process_outgoing(data);
        
        assert!(!result.modified);
        assert_eq!(result.protocol, DetectedProtocol::Unknown);
        assert_eq!(result.fragments.len(), 1);
        assert_eq!(&result.fragments[0][..], &data[..]);
    }
}
