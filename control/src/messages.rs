use serde::{Deserialize, Serialize};

use engine::Config;
use engine::stats::StatsSnapshot;

pub const API_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,    
    pub command: Command,
}

impl Request {
    pub fn new(id: u64, command: Command) -> Self {
        Self { id, command }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Health,    
    Start,    
    Stop,    
    GetConfig,    
    SetConfig(Config),    
    Reload(Config),    
    GetStats,    
    ResetStats,
    GetStatus,    
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,    
    pub success: bool,    
    #[serde(flatten)]
    pub data: ResponseData,
}

impl Response {
    pub fn success(id: u64, data: ResponseData) -> Self {
        Self {
            id,
            success: true,
            data,
        }
    }

    pub fn error(id: u64, message: String) -> Self {
        Self {
            id,
            success: false,
            data: ResponseData::Error { message },
        }
    }

    pub fn ok(id: u64) -> Self {
        Self::success(id, ResponseData::Ok)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum ResponseData {
    Ok,
    Error { message: String },
    Health(HealthInfo),    
    Config(Config),    
    Stats(StatsSnapshot),    
    Status(Status),    
    Pong { timestamp: u64 },    
    Validation { valid: bool, errors: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    pub running: bool,    
    pub version: String,    
    pub api_version: String,    
    pub uptime_secs: u64,    
    pub backend: Option<String>,    
    pub system: SystemInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,    
    pub arch: String,    
    pub rust_version: String,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,    
    pub state: EngineState,    
    pub active_flows: u64,    
    pub packets_processed: u64,    
    pub bytes_processed: u64,    
    pub error_count: u64,    
    pub last_error: Option<String>,    
    pub config_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Stopped,
    Starting,    
    Running,    
    Stopping,    
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub kind: NotificationKind,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "notification", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    StateChanged { old: EngineState, new: EngineState },    
    ConfigReloaded,    
    Error { message: String },
    StatsUpdate(StatsSnapshot),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = Request::new(1, Command::Health);
        let json = serde_json::to_string(&request).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        
        assert_eq!(parsed.id, 1);
        assert!(matches!(parsed.command, Command::Health));
    }

    #[test]
    fn test_response_success() {
        let response = Response::ok(42);
        assert!(response.success);
        assert_eq!(response.id, 42);
    }

    #[test]
    fn test_response_error() {
        let response = Response::error(42, "test error".to_string());
        assert!(!response.success);
        
        if let ResponseData::Error { message } = response.data {
            assert_eq!(message, "test error");
        } else {
            panic!("expected Ererrror variant");
        }
    }

    #[test]
    fn test_command_variants() {
        let commands = vec![
            Command::Health,
            Command::Start,
            Command::Stop,
            Command::GetConfig,
            Command::GetStats,
            Command::GetStatus,
            Command::Ping,
        ];
        
        for cmd in commands {
            let json = serde_json::to_string(&cmd).unwrap();
            let _: Command = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_health_info() {
        let health = HealthInfo {
            running: true,
            version: "0.1.0".to_string(),
            api_version: API_VERSION.to_string(),
            uptime_secs: 3600,
            backend: Some("proxy".to_string()),
            system: SystemInfo::default(),
        };
        
        let json = serde_json::to_string(&health).unwrap();
        let parsed: HealthInfo = serde_json::from_str(&json).unwrap();
        
        assert!(parsed.running);
        assert_eq!(parsed.uptime_secs, 3600);
    }

    #[test]
    fn test_status() {
        let status = Status {
            running: true,
            state: EngineState::Running,
            active_flows: 100,
            packets_processed: 10000,
            bytes_processed: 1_000_000,
            error_count: 0,
            last_error: None,
            config_path: Some("/etc/turkeydpi/config.toml".to_string()),
        };
        
        let json = serde_json::to_string(&status).unwrap();
        let parsed: Status = serde_json::from_str(&json).unwrap();
        
        assert_eq!(parsed.state, EngineState::Running);
        assert_eq!(parsed.active_flows, 100);
    }
}
