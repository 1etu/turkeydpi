use bytes::BytesMut;
use tracing::trace;

use crate::config::{PaddingParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct PaddingTransform {
    params: PaddingParams,
}

impl PaddingTransform {
    pub fn new(params: &PaddingParams) -> Self {
        Self {
            params: params.clone(),
        }
    }

    fn calculate_padding_size(&self, seed: u64) -> usize {
        if self.params.max_bytes == 0 {
            return 0;
        }

        let range = self.params.max_bytes - self.params.min_bytes;
        if range == 0 {
            return self.params.min_bytes;
        }

        
        self.params.min_bytes + ((seed as usize) % (range + 1))
    }

    fn generate_padding(&self, size: usize, seed: u64) -> Vec<u8> {
        match self.params.fill_byte {
            Some(byte) => vec![byte; size],
            None => {
                
                let mut padding = Vec::with_capacity(size);
                let mut value = seed;
                for _ in 0..size {
                    value = value.wrapping_mul(1103515245).wrapping_add(12345);
                    padding.push((value >> 16) as u8);
                }
                padding
            }
        }
    }
}

impl Transform for PaddingTransform {
    fn name(&self) -> &'static str {
        "padding"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        if self.params.max_bytes == 0 {
            return Ok(TransformResult::Continue);
        }

        
        let seed = ctx.state.packet_count
            .wrapping_mul(48271)
            .wrapping_add(data.len() as u64);

        let padding_size = self.calculate_padding_size(seed);
        
        if padding_size == 0 {
            return Ok(TransformResult::Continue);
        }

        let padding = self.generate_padding(padding_size, seed);
        
        trace!(
            flow = ?ctx.key,
            original_size = data.len(),
            padding_size = padding_size,
            "adding padding"
        );

        data.extend_from_slice(&padding);

        Ok(TransformResult::Continue)
    }

    fn is_enabled(&self, params: &TransformParams) -> bool {
        params.padding.max_bytes > 0
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

    #[test]
    fn test_padding_disabled() {
        let params = PaddingParams {
            min_bytes: 0,
            max_bytes: 0,
            fill_byte: None,
        };
        let transform = PaddingTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"test data"[..]);
        let original_len = data.len();

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert_eq!(data.len(), original_len);
    }

    #[test]
    fn test_padding_fixed_size() {
        let params = PaddingParams {
            min_bytes: 10,
            max_bytes: 10,
            fill_byte: Some(0xAB),
        };
        let transform = PaddingTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let original = b"test data";
        let mut data = BytesMut::from(&original[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert_eq!(data.len(), original.len() + 10);
        
        
        for i in original.len()..data.len() {
            assert_eq!(data[i], 0xAB);
        }
    }

    #[test]
    fn test_padding_random_fill() {
        let params = PaddingParams {
            min_bytes: 5,
            max_bytes: 5,
            fill_byte: None,
        };
        let transform = PaddingTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let original = b"test";
        let mut data = BytesMut::from(&original[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert_eq!(data.len(), original.len() + 5);
    }

    #[test]
    fn test_padding_preserves_original() {
        let params = PaddingParams {
            min_bytes: 20,
            max_bytes: 20,
            fill_byte: Some(0x00),
        };
        let transform = PaddingTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let original = b"Hello, World!";
        let mut data = BytesMut::from(&original[..]);

        transform.apply(&mut ctx, &mut data).unwrap();
        
        
        assert_eq!(&data[..original.len()], original);
    }

    #[test]
    fn test_padding_range() {
        let params = PaddingParams {
            min_bytes: 5,
            max_bytes: 15,
            fill_byte: None,
        };
        let transform = PaddingTransform::new(&params);
        
        
        for seed in 0..100u64 {
            let size = transform.calculate_padding_size(seed);
            assert!(size >= 5);
            assert!(size <= 15);
        }
    }
}
