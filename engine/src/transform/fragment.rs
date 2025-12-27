use bytes::BytesMut;
use tracing::{debug, trace};

use crate::config::{FragmentParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct FragmentTransform {
    params: FragmentParams,
}

impl FragmentTransform {
    pub fn new(params: &FragmentParams) -> Self {
        Self {
            params: params.clone(),
        }
    }

    fn calculate_fragment_size(&self, remaining: usize) -> usize {
        if self.params.randomize {
            let range = self.params.max_size - self.params.min_size;
            if range == 0 {
                self.params.min_size
            } else {
                let pseudo_random = (remaining * 31337) % (range + 1);
                self.params.min_size + pseudo_random
            }
        } else {
            self.params.max_size
        }
    }

    pub fn fragment_data(&self, data: &[u8]) -> Vec<BytesMut> {
        let mut fragments = Vec::new();
        let mut offset = 0;
        
        if let Some(split_at) = self.params.split_at_offset {
            if split_at > 0 && split_at < data.len() {
                let first = BytesMut::from(&data[..split_at]);
                let second = BytesMut::from(&data[split_at..]);
                fragments.push(first);
                fragments.push(second);
                return fragments;
            }
        }     
        while offset < data.len() {
            let remaining = data.len() - offset;
            let size = self.calculate_fragment_size(remaining).min(remaining);
            
            let fragment = BytesMut::from(&data[offset..offset + size]);
            fragments.push(fragment);
            offset += size;
        }

        fragments
    }
}

impl Transform for FragmentTransform {
    fn name(&self) -> &'static str {
        "fragment"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        
        if data.len() <= self.params.min_size {
            trace!(
                flow = ?ctx.key,
                size = data.len(),
                "packet too small to fragment"
            );
            return Ok(TransformResult::Continue);
        }

        let fragments = self.fragment_data(data);
        
        if fragments.len() <= 1 {
            return Ok(TransformResult::Continue);
        }

        debug!(
            flow = ?ctx.key,
            original_size = data.len(),
            fragments = fragments.len(),
            "fragmented packet"
        );

        
        ctx.state.transform_state.fragment.fragments_generated += fragments.len() as u32;

        
        for (i, fragment) in fragments.into_iter().enumerate() {
            if i == 0 {
                
                data.clear();
                data.extend_from_slice(&fragment);
            } else {
                ctx.emit(fragment);
            }
        }

        Ok(TransformResult::Fragmented)
    }

    fn is_enabled(&self, _params: &TransformParams) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use crate::config::Protocol;
    use crate::flow::{FlowKey, FlowState};

    fn test_context<'a>(key: &'a FlowKey, state: &'a mut FlowState) -> FlowContext<'a> {
        FlowContext::new(key, state, None)
    }

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
    fn test_fragment_basic() {
        let params = FragmentParams {
            min_size: 5,
            max_size: 10,
            split_at_offset: None,
            randomize: false,
        };
        let transform = FragmentTransform::new(&params);

        let data = b"Hello, this is a test message that should be fragmented";
        let fragments = transform.fragment_data(data);

        assert!(fragments.len() > 1);
        
        
        let reassembled: Vec<u8> = fragments.iter().flat_map(|f| f.iter().copied()).collect();
        assert_eq!(reassembled.as_slice(), data);
    }

    #[test]
    fn test_fragment_small_packet() {
        let params = FragmentParams {
            min_size: 10,
            max_size: 20,
            split_at_offset: None,
            randomize: false,
        };
        let transform = FragmentTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = test_context(&key, &mut state);
        let mut data = BytesMut::from(&b"small"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.output_packets.is_empty());
    }

    #[test]
    fn test_fragment_split_at_offset() {
        let params = FragmentParams {
            min_size: 1,
            max_size: 100,
            split_at_offset: Some(5),
            randomize: false,
        };
        let transform = FragmentTransform::new(&params);

        let data = b"Hello, World!";
        let fragments = transform.fragment_data(data);

        assert_eq!(fragments.len(), 2);
        assert_eq!(&fragments[0][..], b"Hello");
        assert_eq!(&fragments[1][..], b", World!");
    }

    #[test]
    fn test_fragment_apply() {
        let params = FragmentParams {
            min_size: 1,
            max_size: 5,
            split_at_offset: None,
            randomize: false,
        };
        let transform = FragmentTransform::new(&params);

        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = test_context(&key, &mut state);
        let mut data = BytesMut::from(&b"This is a longer test message"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Fragmented);
        
        
        assert!(data.len() <= 5);
        assert!(!ctx.output_packets.is_empty());
    }

    #[test]
    fn test_fragment_preserves_all_data() {
        let params = FragmentParams {
            min_size: 3,
            max_size: 7,
            split_at_offset: None,
            randomize: false,
        };
        let transform = FragmentTransform::new(&params);

        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = test_context(&key, &mut state);
        let original = b"The quick brown fox jumps over the lazy dog";
        let mut data = BytesMut::from(&original[..]);

        let _ = transform.apply(&mut ctx, &mut data);

        
        let mut all_data = data.to_vec();
        for packet in &ctx.output_packets {
            all_data.extend_from_slice(packet);
        }

        assert_eq!(all_data.as_slice(), original);
    }
}
