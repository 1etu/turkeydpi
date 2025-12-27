use std::time::Duration;

use bytes::BytesMut;
use tracing::trace;

use crate::config::{JitterParams, TransformParams};
use crate::error::Result;
use crate::flow::FlowContext;
use super::{Transform, TransformResult};

pub struct JitterTransform {
    params: JitterParams,
}

impl JitterTransform {
    pub fn new(params: &JitterParams) -> Self {
        Self {
            params: params.clone(),
        }
    }
    
    fn calculate_jitter(&self, seed: u64) -> Duration {
        if self.params.max_ms == 0 {
            return Duration::ZERO;
        }

        let range = self.params.max_ms - self.params.min_ms;
        if range == 0 {
            return Duration::from_millis(self.params.min_ms);
        }

        
        let jitter_ms = self.params.min_ms + (seed % (range + 1));
        Duration::from_millis(jitter_ms)
    }
}

impl Transform for JitterTransform {
    fn name(&self) -> &'static str {
        "jitter"
    }

    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult> {
        
        if self.params.max_ms == 0 {
            return Ok(TransformResult::Continue);
        }

        
        let seed = ctx.state.packet_count
            .wrapping_mul(31337)
            .wrapping_add(data.len() as u64);
        
        let jitter = self.calculate_jitter(seed);

        if jitter.is_zero() {
            return Ok(TransformResult::Continue);
        }

        trace!(
            flow = ?ctx.key,
            jitter_ms = jitter.as_millis(),
            "applying jitter"
        );

        
        ctx.state.transform_state.jitter.last_jitter_ms = jitter.as_millis() as u64;
        ctx.state.transform_state.jitter.total_jitter_ms += jitter.as_millis() as u64;

        ctx.request_delay(jitter);
        Ok(TransformResult::Delay)
    }

    fn is_enabled(&self, params: &TransformParams) -> bool {
        params.jitter.max_ms > 0
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
    fn test_jitter_disabled() {
        let params = JitterParams {
            min_ms: 0,
            max_ms: 0,
        };
        let transform = JitterTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"test data"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Continue);
        assert!(ctx.delay.is_none());
    }

    #[test]
    fn test_jitter_applied() {
        let params = JitterParams {
            min_ms: 10,
            max_ms: 50,
        };
        let transform = JitterTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"test data"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Delay);
        
        let delay = ctx.delay.unwrap();
        assert!(delay >= Duration::from_millis(10));
        assert!(delay <= Duration::from_millis(50));
    }

    #[test]
    fn test_jitter_fixed() {
        let params = JitterParams {
            min_ms: 25,
            max_ms: 25,
        };
        let transform = JitterTransform::new(&params);
        
        let key = test_flow_key();
        let mut state = FlowState::new(key);
        let mut ctx = FlowContext::new(&key, &mut state, None);
        let mut data = BytesMut::from(&b"test data"[..]);

        let result = transform.apply(&mut ctx, &mut data).unwrap();
        assert_eq!(result, TransformResult::Delay);
        assert_eq!(ctx.delay.unwrap(), Duration::from_millis(25));
    }

    #[test]
    fn test_jitter_bounds() {
        let params = JitterParams {
            min_ms: 0,
            max_ms: 100,
        };
        let transform = JitterTransform::new(&params);
        
        
        for seed in 0..100 {
            let jitter = transform.calculate_jitter(seed);
            assert!(jitter <= Duration::from_millis(100));
        }
    }
}
