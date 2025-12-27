use bytes::BytesMut;
use tracing::trace;

use crate::config::{HeaderParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct HeaderNormalizationTransform {
    params: HeaderParams,
}

impl HeaderNormalizationTransform {
    pub fn new(params: &HeaderParams) -> Self {
        Self {
            params: params.clone(),
        }
    }

    fn normalize_ipv4(&self, data: &mut BytesMut, seed: u64) {
        if data.len() < 20 {
            return; 
        }

        
        let version = (data[0] >> 4) & 0x0F;
        if version != 4 {
            return;
        }

        
        if self.params.normalize_ttl {
            data[8] = self.params.ttl_value;
        }

        
        if self.params.randomize_ip_id {
            let new_id = ((seed >> 16) as u16).to_be_bytes();
            data[4] = new_id[0];
            data[5] = new_id[1];
        }
    }

    fn tcp_offset(&self, data: &[u8]) -> Option<usize> {
        if data.len() < 20 {
            return None;
        }

        let version = (data[0] >> 4) & 0x0F;
        if version != 4 {
            return None;
        }

        
        if data[9] != 6 {
            return None;
        }

        
        let ihl = (data[0] & 0x0F) as usize * 4;
        if data.len() < ihl + 20 {
            return None;
        }

        Some(ihl)
    }

    fn normalize_tcp(&self, data: &mut BytesMut) {
        let tcp_offset = match self.tcp_offset(data) {
            Some(offset) => offset,
            None => return,
        };

        if self.params.normalize_window {
            
            let window = 65535u16.to_be_bytes();
            data[tcp_offset + 14] = window[0];
            data[tcp_offset + 15] = window[1];
        }
    }
}

impl Transform for HeaderNormalizationTransform {
    fn name(&self) -> &'static str {
        "header_normalization"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        
        let seed = ctx.state.packet_count.wrapping_mul(0xDEADBEEF);

        trace!(
            flow = ?ctx.key,
            size = data.len(),
            "normalizing headers"
        );

        self.normalize_ipv4(data, seed);
        self.normalize_tcp(data);

        Ok(TransformResult::Continue)
    }

    fn is_enabled(&self, params: &TransformParams) -> bool {
        params.header.normalize_ttl 
            || params.header.normalize_window 
            || params.header.randomize_ip_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use crate::config::Protocol;
    use crate::flow::{FlowKey, FlowState};

    fn test_flow_key() -> FlowKey {
        FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            12345,
            443,
            Protocol::Tcp,
        )
    }

    fn create_ipv4_header() -> BytesMut {
        let mut header = BytesMut::with_capacity(40);
        
        
        header.extend_from_slice(&[
            0x45,       
            0x00,       
            0x00, 0x28, 
            0x12, 0x34, 
            0x00, 0x00, 
            0x40,       
            0x06,       
            0x00, 0x00, 
            192, 168, 1, 1,  
            8, 8, 8, 8,      
        ]);
        
        
        header.extend_from_slice(&[
            0x30, 0x39, 
            0x01, 0xBB, 
            0x00, 0x00, 0x00, 0x01, 
            0x00, 0x00, 0x00, 0x00, 
            0x50, 0x02, 
            0x72, 0x10, 
            0x00, 0x00, 
            0x00, 0x00, 
        ]);
        
        header
    }

    #[test]
    fn test_normalize_ttl() {
        let params = HeaderParams {
            normalize_ttl: true,
            ttl_value: 128,
            normalize_window: false,
            randomize_ip_id: false,
        };
        let transform = HeaderNormalizationTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_header();

        
        assert_eq!(data[8], 0x40);

        transform.apply(&mut ctx, &mut data).unwrap();

        
        assert_eq!(data[8], 128);
    }

    #[test]
    fn test_randomize_ip_id() {
        let params = HeaderParams {
            normalize_ttl: false,
            ttl_value: 64,
            normalize_window: false,
            randomize_ip_id: true,
        };
        let transform = HeaderNormalizationTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_header();

        
        let original_id = [data[4], data[5]];

        transform.apply(&mut ctx, &mut data).unwrap();

        
        let new_id = [data[4], data[5]];
        
        assert_ne!(original_id, new_id);
    }

    #[test]
    fn test_normalize_window() {
        let params = HeaderParams {
            normalize_ttl: false,
            ttl_value: 64,
            normalize_window: true,
            randomize_ip_id: false,
        };
        let transform = HeaderNormalizationTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_header();

        
        let tcp_window_offset = 20 + 14;
        
        transform.apply(&mut ctx, &mut data).unwrap();

        
        assert_eq!(data[tcp_window_offset], 0xFF);
        assert_eq!(data[tcp_window_offset + 1], 0xFF);
    }

    #[test]
    fn test_small_packet_ignored() {
        let params = HeaderParams {
            normalize_ttl: true,
            ttl_value: 128,
            normalize_window: true,
            randomize_ip_id: true,
        };
        let transform = HeaderNormalizationTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"small"[..]);

        
        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
    }

    #[test]
    fn test_non_ipv4_ignored() {
        let params = HeaderParams {
            normalize_ttl: true,
            ttl_value: 128,
            normalize_window: false,
            randomize_ip_id: false,
        };
        let transform = HeaderNormalizationTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        
        
        let mut data = BytesMut::from(&[0x60u8; 40][..]);

        let original = data.clone();
        transform.apply(&mut ctx, &mut data).unwrap();
        
        
        assert_eq!(data[..], original[..]);
    }
}
