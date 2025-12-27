use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use engine::{Config, Stats};
use backend::{Backend, BackendHandle, BackendConfig, BackendSettings, ProxySettings};
use backend::proxy::ProxyBackend;

use crate::error::{ControlError, Result};
use crate::messages::{
    Command, EngineState, HealthInfo,
    Request, Response, ResponseData, Status, SystemInfo, API_VERSION,
};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub socket_path: PathBuf,    
    pub max_clients: usize,    
    pub timeout_secs: u64,    
    pub enable_notifications: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/tmp/turkeydpi.sock"),
            max_clients: 10,
            timeout_secs: 30,
            enable_notifications: true,
        }
    }
}

struct ServerState {
    config: RwLock<Config>,    
    backend_handle: RwLock<Option<BackendHandle>>,    
    engine_state: RwLock<EngineState>,    
    start_time: Instant,    
    backend_type: RwLock<Option<String>>,    
    last_error: RwLock<Option<String>>,    
    config_path: RwLock<Option<PathBuf>>,
}

impl ServerState {
    fn new(config: Config) -> Self {
        Self {
            config: RwLock::new(config),
            backend_handle: RwLock::new(None),
            engine_state: RwLock::new(EngineState::Stopped),
            start_time: Instant::now(),
            backend_type: RwLock::new(None),
            last_error: RwLock::new(None),
            config_path: RwLock::new(None),
        }
    }
}

pub struct ControlServer {
    server_config: ServerConfig,    
    running: Arc<AtomicBool>,    
    state: Arc<ServerState>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl ControlServer {
    pub fn new(server_config: ServerConfig, engine_config: Config) -> Self {
        Self {
            server_config,
            running: Arc::new(AtomicBool::new(false)),
            state: Arc::new(ServerState::new(engine_config)),
            shutdown_tx: None,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(ControlError::AlreadyRunning);
        }

        let socket_path = &self.server_config.socket_path;
        
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        info!(socket = %socket_path.display(), "Starting control server");

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| ControlError::BindFailed(e.to_string()))?;

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let state = self.state.clone();
        let max_clients = self.server_config.max_clients;

        tokio::spawn(async move {
            let mut active_clients = 0usize;
            
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Control server received shutdown signal");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _addr)) => {
                                if active_clients >= max_clients {
                                    warn!("Max clients reached, rejecting connection");
                                    continue;
                                }
                                
                                active_clients += 1;
                                let state = state.clone();
                                
                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_client(stream, state).await {
                                        debug!(error = %e, "Client handler error");
                                    }
                                });
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to accept connection");
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("Control server stopped");
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ControlError::NotRunning);
        }

        info!("Stopping control server");

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        let _ = std::fs::remove_file(&self.server_config.socket_path);

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn handle_client(stream: UnixStream, state: Arc<ServerState>) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            
            let bytes_read = reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            trace!(request = %line, "Received request");

            let response = match serde_json::from_str::<Request>(line) {
                Ok(request) => Self::handle_request(&request, &state).await,
                Err(e) => Response::error(0, format!("Invalid JSON: {}", e)),
            };

            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(request: &Request, state: &ServerState) -> Response {
        let id = request.id;

        match &request.command {
            Command::Health => {
                let health = HealthInfo {
                    running: *state.engine_state.read() == EngineState::Running,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    api_version: API_VERSION.to_string(),
                    uptime_secs: state.start_time.elapsed().as_secs(),
                    backend: state.backend_type.read().clone(),
                    system: SystemInfo::default(),
                };
                Response::success(id, ResponseData::Health(health))
            }

            Command::Start => {
                let current_state = *state.engine_state.read();
                if current_state == EngineState::Running {
                    return Response::error(id, "Engine already running".to_string());
                }

                *state.engine_state.write() = EngineState::Starting;

                let config = state.config.read().clone();
                let backend_config = BackendConfig {
                    engine_config: config,
                    max_queue_size: 1000,
                    backend_settings: BackendSettings::Proxy(
                        ProxySettings::default()
                    ),
                };

                let mut backend = ProxyBackend::new();
                match backend.start(backend_config).await {
                    Ok(handle) => {
                        *state.backend_handle.write() = Some(handle);
                        *state.backend_type.write() = Some("proxy".to_string());
                        *state.engine_state.write() = EngineState::Running;
                        *state.last_error.write() = None;
                        Response::ok(id)
                    }
                    Err(e) => {
                        *state.engine_state.write() = EngineState::Error;
                        *state.last_error.write() = Some(e.to_string());
                        Response::error(id, e.to_string())
                    }
                }
            }

            Command::Stop => {
                let current_state = *state.engine_state.read();
                if current_state != EngineState::Running {
                    return Response::error(id, "Engine not running".to_string());
                }

                *state.engine_state.write() = EngineState::Stopping;

                let handle = state.backend_handle.write().take();
                if let Some(handle) = handle {
                    if let Err(e) = handle.shutdown().await {
                        warn!(error = %e, "Error during shutdown");
                    }
                }

                *state.backend_type.write() = None;
                *state.engine_state.write() = EngineState::Stopped;
                Response::ok(id)
            }

            Command::GetConfig => {
                let config = state.config.read().clone();
                Response::success(id, ResponseData::Config(config))
            }

            Command::SetConfig(new_config) => {
                match new_config.validate() {
                    Ok(()) => {
                        Response::success(id, ResponseData::Validation {
                            valid: true,
                            errors: vec![],
                        })
                    }
                    Err(e) => {
                        Response::success(id, ResponseData::Validation {
                            valid: false,
                            errors: vec![e.to_string()],
                        })
                    }
                }
            }

            Command::Reload(new_config) => {
                if let Err(e) = new_config.validate() {
                    return Response::error(id, e.to_string());
                }

                *state.config.write() = new_config.clone();

                if let Some(ref handle) = *state.backend_handle.read() {
                    if let Err(e) = handle.reload_config(new_config.clone()) {
                        return Response::error(id, e.to_string());
                    }
                }

                Response::ok(id)
            }

            Command::GetStats => {
                let stats = if let Some(ref handle) = *state.backend_handle.read() {
                    handle.stats().snapshot()
                } else {
                    Stats::new().snapshot()
                };
                Response::success(id, ResponseData::Stats(stats))
            }

            Command::ResetStats => {
                if let Some(ref handle) = *state.backend_handle.read() {
                    handle.stats().reset();
                }
                Response::ok(id)
            }

            Command::GetStatus => {
                let backend_handle = state.backend_handle.read();
                let (active_flows, packets, bytes, errors) = if let Some(ref handle) = *backend_handle {
                    let s = handle.stats().snapshot();
                    (s.active_flows, s.packets_in, s.bytes_in, s.transform_errors)
                } else {
                    (0, 0, 0, 0)
                };

                let status = Status {
                    running: *state.engine_state.read() == EngineState::Running,
                    state: *state.engine_state.read(),
                    active_flows,
                    packets_processed: packets,
                    bytes_processed: bytes,
                    error_count: errors,
                    last_error: state.last_error.read().clone(),
                    config_path: state.config_path.read().as_ref().map(|p| p.display().to_string()),
                };
                Response::success(id, ResponseData::Status(status))
            }

            Command::Ping => {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                Response::success(id, ResponseData::Pong { timestamp })
            }
        }
    }

    pub fn load_config(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let config = Config::load_from_file(path)?;
        
        *self.state.config.write() = config;
        *self.state.config_path.write() = Some(path.to_path_buf());
        
        info!(path = %path.display(), "Loaded configuration");
        Ok(())
    }

    pub fn socket_path(&self) -> &Path {
        &self.server_config.socket_path
    }
}

pub struct ControlClient {
    socket_path: PathBuf,
    next_id: u64,
}

impl ControlClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            next_id: 1,
        }
    }

    pub async fn send(&mut self, command: Command) -> Result<Response> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| ControlError::Connection(e.to_string()))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let request = Request::new(self.next_id, command);
        self.next_id += 1;

        let request_json = serde_json::to_string(&request)?;
        writer.write_all(request_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        let mut line = String::new();
        reader.read_line(&mut line).await?;

        let response: Response = serde_json::from_str(&line)?;
        Ok(response)
    }

    pub async fn health(&mut self) -> Result<HealthInfo> {
        let response = self.send(Command::Health).await?;
        match response.data {
            ResponseData::Health(info) => Ok(info),
            ResponseData::Error { message } => Err(ControlError::Internal(message)),
            _ => Err(ControlError::InvalidRequest("Unexpected response".to_string())),
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let response = self.send(Command::Start).await?;
        if response.success {
            Ok(())
        } else if let ResponseData::Error { message } = response.data {
            Err(ControlError::Internal(message))
        } else {
            Err(ControlError::Internal("Unknown error".to_string()))
        }
    }

    pub async fn stop(&mut self) -> Result<()> {
        let response = self.send(Command::Stop).await?;
        if response.success {
            Ok(())
        } else if let ResponseData::Error { message } = response.data {
            Err(ControlError::Internal(message))
        } else {
            Err(ControlError::Internal("Unknown error".to_string()))
        }
    }

    pub async fn status(&mut self) -> Result<Status> {
        let response = self.send(Command::GetStatus).await?;
        match response.data {
            ResponseData::Status(status) => Ok(status),
            ResponseData::Error { message } => Err(ControlError::Internal(message)),
            _ => Err(ControlError::InvalidRequest("Unexpected response".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_server_start_stop() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");
        
        let server_config = ServerConfig {
            socket_path: socket_path.clone(),
            ..Default::default()
        };
        
        let mut server = ControlServer::new(server_config, Config::default());
        
        server.start().await.unwrap();
        assert!(server.is_running());
        
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        
        server.stop().await.unwrap();
        assert!(!server.is_running());
    }

    #[tokio::test]
    async fn test_client_server_communication() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");
        
        let server_config = ServerConfig {
            socket_path: socket_path.clone(),
            ..Default::default()
        };
        
        let mut server = ControlServer::new(server_config, Config::default());
        server.start().await.unwrap();
        
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        let mut client = ControlClient::new(&socket_path);
        let health = client.health().await.unwrap();
        
        assert!(!health.running);
        assert_eq!(health.api_version, API_VERSION);
        
        let status = client.status().await.unwrap();
        assert_eq!(status.state, EngineState::Stopped);
        
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_ping_pong() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");
        
        let server_config = ServerConfig {
            socket_path: socket_path.clone(),
            ..Default::default()
        };
        
        let mut server = ControlServer::new(server_config, Config::default());
        server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        let mut client = ControlClient::new(&socket_path);
        let response = client.send(Command::Ping).await.unwrap();
        
        assert!(response.success);
        if let ResponseData::Pong { timestamp } = response.data {
            assert!(timestamp > 0);
        } else {
            panic!("Expected Pong response");
        }
        
        server.stop().await.unwrap();
    }
}
