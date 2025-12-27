use bytes::BytesMut;
use tracing::trace;

use crate::config::{ResegmentParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct ResegmentTransform {
    params: ResegmentParams,
}

impl ResegmentTransform {
    pub fn new(params: &ResegmentParams) -> Self {
        Self {
            params: params.clone(),
        }
    }

    pub fn segment_data(&self, data: &[u8]) -> Vec<BytesMut> {
        let mut segments = Vec::new();
        let mut offset = 0;
        let mut count = 0;

        while offset < data.len() && count < self.params.max_segments {
            let remaining = data.len() - offset;
            let size = self.params.segment_size.min(remaining);
            
            let segment = BytesMut::from(&data[offset..offset + size]);
            segments.push(segment);
            
            offset += size;
            count += 1;
        }

        
        if offset < data.len() {
            segments.push(BytesMut::from(&data[offset..]));
        }

        segments
    }
}

impl Transform for ResegmentTransform {
    fn name(&self) -> &'static str {
        "resegment"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        
        if data.len() <= self.params.segment_size {
            return Ok(TransformResult::Continue);
        }

        let segments = self.segment_data(data);
        
        if segments.len() <= 1 {
            return Ok(TransformResult::Continue);
        }

        trace!(
            flow = ?ctx.key,
            original_size = data.len(),
            segments = segments.len(),
            "resegmented packet"
        );

        
        ctx.state.transform_state.resegment.segments_generated += segments.len() as u32;

        
        for (i, segment) in segments.into_iter().enumerate() {
            if i == 0 {
                data.clear();
                data.extend_from_slice(&segment);
            } else {
                ctx.emit(segment);
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
    fn test_resegment_basic() {
        let params = ResegmentParams {
            segment_size: 10,
            max_segments: 100,
        };
        let transform = ResegmentTransform::new(&params);

        let data = b"This is a test message for resegmentation";
        let segments = transform.segment_data(data);

        
        for (i, segment) in segments.iter().enumerate() {
            if i < segments.len() - 1 {
                assert_eq!(segment.len(), 10);
            }
        }

        
        let reassembled: Vec<u8> = segments.iter().flat_map(|s| s.iter().copied()).collect();
        assert_eq!(reassembled.as_slice(), data);
    }

    #[test]
    fn test_resegment_max_segments() {
        let params = ResegmentParams {
            segment_size: 5,
            max_segments: 3,
        };
        let transform = ResegmentTransform::new(&params);

        let data = b"12345678901234567890"; 
        let segments = transform.segment_data(data);

        
        assert_eq!(segments.len(), 4);
        
        
        let reassembled: Vec<u8> = segments.iter().flat_map(|s| s.iter().copied()).collect();
        assert_eq!(reassembled.as_slice(), data);
    }

    #[test]
    fn test_resegment_small_packet() {
        let params = ResegmentParams {
            segment_size: 20,
            max_segments: 10,
        };
        let transform = ResegmentTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"small"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.output_packets.is_empty());
    }

    #[test]
    fn test_resegment_apply() {
        let params = ResegmentParams {
            segment_size: 8,
            max_segments: 100,
        };
        let transform = ResegmentTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let original = b"The quick brown fox jumps over the lazy dog";
        let mut data = BytesMut::from(&original[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Fragmented);
        
        
        assert!(data.len() <= 8);
        assert!(!ctx.output_packets.is_empty());
        
        
        let mut all_data = data.to_vec();
        for packet in &ctx.output_packets {
            all_data.extend_from_slice(packet);
        }
        assert_eq!(all_data.as_slice(), original);
    }
}
