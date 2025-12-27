pub mod fragment;
pub mod jitter;
pub mod padding;
pub mod header;
pub mod resegment;
pub mod decoy;

use bytes::BytesMut;
use serde::{Deserialize, Serialize};

use crate::config::TransformParams;
use crate::error::Result;
use crate::flow::FlowContext;

pub use fragment::FragmentTransform;
pub use jitter::JitterTransform;
pub use padding::PaddingTransform;
pub use header::HeaderNormalizationTransform;
pub use resegment::ResegmentTransform;
pub use decoy::DecoyTransform;

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
        Box::new(PaddingTransform::new(&params.padding)),
        Box::new(JitterTransform::new(&params.jitter)),
        Box::new(HeaderNormalizationTransform::new(&params.header)),
        Box::new(DecoyTransform::new(&params.decoy)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_all_transforms() {
        let params = TransformParams::default();
        let transforms = create_all_transforms(&params);
        
        assert_eq!(transforms.len(), 6);
        
        let names: Vec<&str> = transforms.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"fragment"));
        assert!(names.contains(&"resegment"));
        assert!(names.contains(&"padding"));
        assert!(names.contains(&"jitter"));
        assert!(names.contains(&"header_normalization"));
        assert!(names.contains(&"decoy"));
    }
}
