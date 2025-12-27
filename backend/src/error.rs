use thiserror::Error;

pub type Result<T> = std::result::Result<T, BackendError>;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("Backend not supported on this platform: {0}")]
    NotSupported(String),

    #[error("Failed to create TUN device: {0}")]
    TunCreationFailed(String),

    #[error("Failed to configure network: {0}")]
    NetworkConfig(String),

    #[error("Backend already running")]
    AlreadyRunning,

    #[error("Backend not running")]
    NotRunning,

    #[error("Failed to bind to address: {0}")]
    BindFailed(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Queue full: {0}")]
    QueueFull(String),

    #[error("Packet too large: {size} bytes (max: {max})")]
    PacketTooLarge { size: usize, max: usize },

    #[error("Invalid packet: {0}")]
    InvalidPacket(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Engine error: {0}")]
    Engine(#[from] engine::EngineError),

    #[error("Shutdown requested")]
    Shutdown,

    #[error("Timeout waiting for operation")]
    Timeout,

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}
