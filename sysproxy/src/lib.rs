use thiserror::Error;

pub type Result<T> = std::result::Result<T, SysProxyError>;

#[derive(Debug, Error)]
pub enum SysProxyError {
    #[error("system proxy is not supported on this platform")]
    Unsupported,

    #[error("permission denied, run with administrator rights")]
    PermissionDenied,

    #[error("{0}")]
    Failed(String),
}

#[cfg(any(windows, target_os = "macos"))]
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
mod imp;

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::{Result, SysProxyError};

    pub fn enable(_host: &str, _port: u16) -> Result<()> {
        Err(SysProxyError::Unsupported)
    }

    pub fn disable() -> Result<()> {
        Err(SysProxyError::Unsupported)
    }

    pub fn is_enabled() -> Result<bool> {
        Err(SysProxyError::Unsupported)
    }
}

pub fn enable(host: &str, port: u16) -> Result<()> {
    imp::enable(host, port)
}

pub fn disable() -> Result<()> {
    imp::disable()
}

pub fn is_enabled() -> Result<bool> {
    imp::is_enabled()
}

pub fn is_supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}
