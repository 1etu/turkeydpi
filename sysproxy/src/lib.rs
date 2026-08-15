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

    pub fn active_server() -> Result<Option<String>> {
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

pub fn active_server() -> Result<Option<String>> {
    imp::active_server()
}

pub fn points_at_loopback(server: &str) -> bool {
    let mut parts = server
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .peekable();

    if parts.peek().is_none() {
        return false;
    }

    parts.all(|part| {
        let value = match part.split_once('=') {
            Some((_, value)) => value.trim(),
            None => part,
        };

        let host = match value.rsplit_once(':') {
            Some((host, _)) => host,
            None => value,
        };

        let host = host.trim_matches(|c| c == '[' || c == ']');

        host.eq_ignore_ascii_case("localhost") || host.starts_with("127.") || host == "::1"
    })
}

pub fn is_supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

#[cfg(test)]
mod tests {
    use super::points_at_loopback;

    #[test]
    fn recognises_our_own_proxy() {
        assert!(points_at_loopback("127.0.0.1:8844"));
        assert!(points_at_loopback("localhost:8844"));
        assert!(points_at_loopback("[::1]:8844"));
        assert!(points_at_loopback(
            "http=127.0.0.1:8844;https=127.0.0.1:8844"
        ));
    }

    #[test]
    fn leaves_someone_elses_proxy_alone() {
        assert!(!points_at_loopback(""));
        assert!(!points_at_loopback("proxy.corp.local:3128"));
        assert!(!points_at_loopback("10.0.0.8:8080"));
        assert!(!points_at_loopback(
            "http=127.0.0.1:8844;https=proxy.corp.local:3128"
        ));
    }
}
