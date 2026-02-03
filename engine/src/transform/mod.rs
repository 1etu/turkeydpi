pub mod fragment;
pub mod jitter;
pub mod resegment;

use bytes::BytesMut;
use serde::{Deserialize, Serialize};

use crate::config::TransformParams;
use crate::error::Result;
use crate::flow::FlowContext;

pub use fragment::FragmentTransform;
pub use jitter::JitterTransform;
pub use resegment::ResegmentTransform;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformResult {
    Continue,
    Fragmented,
    Delay,
    Drop,
    Skip,
    Error(String),
}

pub trait Transform: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, ctx: &mut FlowContext<'_>, data: &mut BytesMut) -> Result<TransformResult>;
    fn is_enabled(&self, params: &TransformParams) -> bool {
        let _ = params;
        true
    }

    fn reset(&self) {}
}

pub type BoxedTransform = Box<dyn Transform>;

pub fn create_all_transforms(params: &TransformParams) -> Vec<BoxedTransform> {
    vec![
        Box::new(FragmentTransform::new(&params.fragment)),
        Box::new(ResegmentTransform::new(&params.resegment)),
        Box::new(JitterTransform::new(&params.jitter)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_all_transforms() {
        let params = TransformParams::default();
        let transforms = create_all_transforms(&params);

        assert_eq!(transforms.len(), 3);

        let names: Vec<&str> = transforms.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"fragment"));
        assert!(names.contains(&"resegment"));
        assert!(names.contains(&"jitter"));
    }
}
