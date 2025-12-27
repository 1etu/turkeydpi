use bytes::BytesMut;
use tracing::trace;

use crate::config::{DecoyParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct DecoyTransform {
    params: DecoyParams,
}

impl DecoyTransform {
    pub fn new(params: &DecoyParams) -> Self {
        Self {
            params: params.clone(),
        }
    }

    fn create_decoy(&self, original: &[u8]) -> Option<BytesMut> {
        if original.len() < 20 {
            return None;
        }

        let version = (original[0] >> 4) & 0x0F;
        if version != 4 {
            return None;
        }

        let mut decoy = BytesMut::from(original);
        
        decoy[8] = self.params.ttl;
        
        if decoy.len() > 5 {
            decoy[4] ^= 0xFF;
            decoy[5] ^= 0xFF;
        }

        Some(decoy)
    }

    fn should_send_decoy(&self, seed: u64) -> bool {
        if self.params.probability <= 0.0 {
            return false;
        }
        if self.params.probability >= 1.0 {
            return true;
        }
        
        let threshold = (self.params.probability * 1000.0) as u64;
        (seed % 1000) < threshold
    }
}

impl Transform for DecoyTransform {
    fn name(&self) -> &'static str {
        "decoy"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        if !self.params.send_before && !self.params.send_after {
            return Ok(TransformResult::Continue);
        }

        let seed = ctx.state.packet_count
            .wrapping_mul(0x1337CAFE)
            .wrapping_add(data.len() as u64);

        if !self.should_send_decoy(seed) {
            return Ok(TransformResult::Continue);
        }

        let decoy = match self.create_decoy(data) {
            Some(d) => d,
            None => return Ok(TransformResult::Continue),
        };

        trace!(
            flow = ?ctx.key,
            ttl = self.params.ttl,
            "generating decoy packet"
        );

        if self.params.send_before {
            let real = data.clone();
            data.clear();
            data.extend_from_slice(&decoy);
            ctx.emit(real);
        }

        if self.params.send_after {
            ctx.emit(decoy);
        }

        Ok(TransformResult::Fragmented)
    }

    fn is_enabled(&self, params: &TransformParams) -> bool {
        params.decoy.probability > 0.0 
            && (params.decoy.send_before || params.decoy.send_after)
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

    fn create_ipv4_packet() -> BytesMut {
        let mut packet = BytesMut::with_capacity(40);
        packet.extend_from_slice(&[
            0x45, 0x00, 0x00, 0x28,
            0x12, 0x34, 0x00, 0x00,
            0x40, 0x06, 0x00, 0x00,
            192, 168, 1, 1,
            8, 8, 8, 8,
            0x30, 0x39, 0x01, 0xBB,
            0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x00,
            0x50, 0x02, 0x72, 0x10,
            0x00, 0x00, 0x00, 0x00,
        ]);
        packet
    }

    #[test]
    fn test_decoy_disabled() {
        let params = DecoyParams {
            send_before: false,
            send_after: false,
            ttl: 1,
            probability: 1.0,
        };
        let transform = DecoyTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_packet();

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.output_packets.is_empty());
    }

    #[test]
    fn test_decoy_probability_zero() {
        let params = DecoyParams {
            send_before: true,
            send_after: true,
            ttl: 1,
            probability: 0.0,
        };
        let transform = DecoyTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_packet();

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.output_packets.is_empty());
    }

    #[test]
    fn test_decoy_send_after() {
        let params = DecoyParams {
            send_before: false,
            send_after: true,
            ttl: 3,
            probability: 1.0,
        };
        let transform = DecoyTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let original = create_ipv4_packet();
        let mut data = original.clone();

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Fragmented);
        assert_eq!(ctx.output_packets.len(), 1);
        
        
        assert_eq!(data[8], 0x40);
        
        
        assert_eq!(ctx.output_packets[0][8], 3);
    }

    #[test]
    fn test_decoy_send_before() {
        let params = DecoyParams {
            send_before: true,
            send_after: false,
            ttl: 2,
            probability: 1.0,
        };
        let transform = DecoyTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = create_ipv4_packet();

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Fragmented);
        assert_eq!(ctx.output_packets.len(), 1);
        
        
        assert_eq!(data[8], 2);
        
        
        assert_eq!(ctx.output_packets[0][8], 0x40);
    }

    #[test]
    fn test_create_decoy_modifies_packet() {
        let params = DecoyParams {
            send_before: false,
            send_after: true,
            ttl: 1,
            probability: 1.0,
        };
        let transform = DecoyTransform::new(&params);

        let original = create_ipv4_packet();
        let decoy = transform.create_decoy(&original).unwrap();

        
        assert_eq!(decoy[8], 1);
        
        
        assert_ne!(decoy[4], original[4]);
        assert_ne!(decoy[5], original[5]);
    }

    #[test]
    fn test_small_packet_no_decoy() {
        let params = DecoyParams {
            send_before: true,
            send_after: true,
            ttl: 1,
            probability: 1.0,
        };
        let transform = DecoyTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"small"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.output_packets.is_empty());
    }
}
