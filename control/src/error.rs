use thiserror::Error;

pub type Result<T> = std::result::Result<T, ControlError>;

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("Server already running")]
    AlreadyRunning,

    #[error("Server not running")]
    NotRunning,

    #[error("Failed to bind to socket: {0}")]
    BindFailed(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("Engine error: {0}")]
    Engine(#[from] engine::EngineError),

    #[error("Backend error: {0}")]
    Backend(#[from] backend::BackendError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Request timeout")]
    Timeout,

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
