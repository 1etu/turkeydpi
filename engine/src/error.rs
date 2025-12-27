use std::net::IpAddr;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Configuration validation failed: {message}")]
    ConfigValidation { message: String, field: String },

    #[error("Transform error in '{transform}': {message}")]
    Transform { transform: String, message: String },

    #[error("Flow limit exceeded: max {max} flows, current {current}")]
    FlowLimitExceeded { max: usize, current: usize },

    #[error("Queue full: {queue_name} (max size: {max_size})")]
    QueueFull { queue_name: String, max_size: usize },

    #[error("Invalid packet: {0}")]
    InvalidPacket(String),

    #[error("Pipeline error: {0}")]
    Pipeline(String),

    #[error("Invalid IP address: {0}")]
    InvalidIpAddr(IpAddr),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("Engine not running")]
    NotRunning,

    #[error("Engine already running")]
    AlreadyRunning,

    #[error("Shutdown requested")]
    Shutdown,
}

impl EngineError {
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ConfigValidation {
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn transform(transform: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Transform {
            transform: transform.into(),
            message: message.into(),
        }
    }
}
