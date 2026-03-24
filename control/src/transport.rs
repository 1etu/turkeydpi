use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub type Listener = tokio::net::UnixListener;

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;

#[cfg(not(unix))]
pub type Listener = tokio::net::TcpListener;

#[cfg(not(unix))]
pub type Stream = tokio::net::TcpStream;

pub fn default_socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp/turkeydpi.sock")
    }

    #[cfg(not(unix))]
    {
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        base.join("turkeydpi").join("control.port")
    }
}

pub async fn bind(path: &Path) -> io::Result<Listener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Listener::bind(path)
    }

    #[cfg(not(unix))]
    {
        let listener = Listener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        std::fs::write(path, port.to_string())?;
        Ok(listener)
    }
}

pub async fn connect(path: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        Stream::connect(path).await
    }

    #[cfg(not(unix))]
    {
        let port: u16 = std::fs::read_to_string(path)?
            .trim()
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid control port file"))?;
        Stream::connect(("127.0.0.1", port)).await
    }
}

pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}
