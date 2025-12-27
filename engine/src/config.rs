use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub global: GlobalConfig,
    
    pub rules: Vec<Rule>,
    
    pub limits: Limits,
    
    pub transforms: TransformParams,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global: GlobalConfig::default(),
            rules: Vec::new(),
            limits: Limits::default(),
            transforms: TransformParams::default(),
        }
    }
}

impl Config {
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        
        let config: Config = if path.extension().map_or(false, |e| e == "toml") {
            toml::from_str(&content)?
        } else {
            serde_json::from_str(&content)?
        };
        
        config.validate()?;
        Ok(config)
    }
    
    pub fn from_json(json: &str) -> Result<Self> {
        let config: Config = serde_json::from_str(json)?;
        config.validate()?;
        Ok(config)
    }
    
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        let config: Config = toml::from_str(toml_str)?;
        config.validate()?;
        Ok(config)
    }
    
    pub fn validate(&self) -> Result<()> {
        
        if self.limits.max_flows == 0 {
            return Err(EngineError::validation("limits.max_flows", "must be > 0"));
        }
        
        if self.limits.max_queue_size == 0 {
            return Err(EngineError::validation("limits.max_queue_size", "must be > 0"));
        }
        
        if self.limits.max_memory_mb == 0 {
            return Err(EngineError::validation("limits.max_memory_mb", "must be > 0"));
        }
        
        
        if self.transforms.fragment.min_size == 0 {
            return Err(EngineError::validation(
                "transforms.fragment.min_size",
                "must be > 0",
            ));
        }
        
        if self.transforms.fragment.max_size < self.transforms.fragment.min_size {
            return Err(EngineError::validation(
                "transforms.fragment.max_size",
                "must be >= min_size",
            ));
        }
        
        if self.transforms.jitter.max_ms > self.limits.max_jitter_ms {
            return Err(EngineError::validation(
                "transforms.jitter.max_ms",
                format!(
                    "exceeds safety limit of {}ms",
                    self.limits.max_jitter_ms
                ),
            ));
        }
        
        if self.transforms.padding.max_bytes > 1500 {
            return Err(EngineError::validation(
                "transforms.padding.max_bytes",
                "exceeds MTU (1500 bytes)",
            ));
        }
        
        
        for (i, rule) in self.rules.iter().enumerate() {
            rule.validate().map_err(|e| {
                EngineError::validation(format!("rules[{}]", i), e.to_string())
            })?;
        }
        
        Ok(())
    }
    
    pub fn merge(&mut self, other: Config) {
        if !other.rules.is_empty() {
            self.rules = other.rules;
        }
        self.global = other.global;
        self.limits = other.limits;
        self.transforms = other.transforms;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub enabled: bool,
    
    pub enable_fragmentation: bool,
    
    pub enable_jitter: bool,
    
    pub enable_padding: bool,
    
    pub enable_header_normalization: bool,
    
    pub log_level: String,
    
    pub json_logging: bool,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enable_fragmentation: true,
            enable_jitter: false,
            enable_padding: true,
            enable_header_normalization: true,
            log_level: "info".to_string(),
            json_logging: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    
    #[serde(default = "default_true")]
    pub enabled: bool,
    
    #[serde(default)]
    pub priority: i32,
    
    pub match_criteria: MatchCriteria,
    
    pub transforms: Vec<TransformType>,
    
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

impl Rule {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(EngineError::validation("name", "cannot be empty"));
        }
        
        if self.transforms.is_empty() {
            return Err(EngineError::validation("transforms", "must specify at least one transform"));
        }
        
        self.match_criteria.validate()?;
        
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchCriteria {
    pub dst_ip: Option<Vec<String>>,
    
    pub src_ip: Option<Vec<String>>,
    
    pub dst_ports: Option<Vec<u16>>,
    
    pub src_ports: Option<Vec<u16>>,
    
    pub protocols: Option<Vec<Protocol>>,
    
    pub domains: Option<Vec<String>>,
    
    pub process: Option<String>,
}

impl MatchCriteria {
    pub fn validate(&self) -> Result<()> {
        
        if let Some(ref ips) = self.dst_ip {
            for ip in ips {
                ip.parse::<IpNet>()
                    .or_else(|_| ip.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| EngineError::validation("dst_ip", format!("invalid IP/CIDR: {}", ip)))?;
            }
        }
        
        if let Some(ref ips) = self.src_ip {
            for ip in ips {
                ip.parse::<IpNet>()
                    .or_else(|_| ip.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| EngineError::validation("src_ip", format!("invalid IP/CIDR: {}", ip)))?;
            }
        }
        
        Ok(())
    }
    
    pub fn is_catch_all(&self) -> bool {
        self.dst_ip.is_none()
            && self.src_ip.is_none()
            && self.dst_ports.is_none()
            && self.src_ports.is_none()
            && self.protocols.is_none()
            && self.domains.is_none()
            && self.process.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformType {
    Fragment,
    
    Resegment,
    
    Padding,
    
    Jitter,
    
    HeaderNormalization,
    
    Decoy,
    
    Reorder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TransformParams {
    pub fragment: FragmentParams,
    
    pub resegment: ResegmentParams,
    
    pub padding: PaddingParams,
    
    pub jitter: JitterParams,
    
    pub header: HeaderParams,
    
    pub decoy: DecoyParams,
}

impl Default for TransformParams {
    fn default() -> Self {
        Self {
            fragment: FragmentParams::default(),
            resegment: ResegmentParams::default(),
            padding: PaddingParams::default(),
            jitter: JitterParams::default(),
            header: HeaderParams::default(),
            decoy: DecoyParams::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FragmentParams {
    pub min_size: usize,
    
    pub max_size: usize,
    
    pub split_at_offset: Option<usize>,
    
    pub randomize: bool,
}

impl Default for FragmentParams {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 40,
            split_at_offset: None,
            randomize: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResegmentParams {
    pub segment_size: usize,
    
    pub max_segments: usize,
}

impl Default for ResegmentParams {
    fn default() -> Self {
        Self {
            segment_size: 16,
            max_segments: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PaddingParams {
    pub min_bytes: usize,
    
    pub max_bytes: usize,
    
    pub fill_byte: Option<u8>,
}

impl Default for PaddingParams {
    fn default() -> Self {
        Self {
            min_bytes: 0,
            max_bytes: 64,
            fill_byte: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JitterParams {
    pub min_ms: u64,
    
    pub max_ms: u64,
}

impl Default for JitterParams {
    fn default() -> Self {
        Self {
            min_ms: 0,
            max_ms: 50,
        }
    }
}

impl JitterParams {
    pub fn as_duration_range(&self) -> (Duration, Duration) {
        (Duration::from_millis(self.min_ms), Duration::from_millis(self.max_ms))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeaderParams {
    pub normalize_ttl: bool,
    
    pub ttl_value: u8,
    
    pub normalize_window: bool,
    
    pub randomize_ip_id: bool,
}

impl Default for HeaderParams {
    fn default() -> Self {
        Self {
            normalize_ttl: false,
            ttl_value: 64,
            normalize_window: false,
            randomize_ip_id: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DecoyParams {
    pub send_before: bool,
    
    pub send_after: bool,
    
    pub ttl: u8,
    
    pub probability: f32,
}

impl Default for DecoyParams {
    fn default() -> Self {
        Self {
            send_before: false,
            send_after: false,
            ttl: 1,
            probability: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_flows: usize,
    
    pub max_queue_size: usize,
    
    pub max_memory_mb: usize,
    
    pub max_jitter_ms: u64,
    
    pub flow_timeout_secs: u64,
    
    pub log_rate_limit: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_flows: 10_000,
            max_queue_size: 1_000,
            max_memory_mb: 128,
            max_jitter_ms: 500,
            flow_timeout_secs: 120,
            log_rate_limit: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_max_flows() {
        let mut config = Config::default();
        config.limits.max_flows = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_fragment_sizes() {
        let mut config = Config::default();
        config.transforms.fragment.max_size = 0;
        config.transforms.fragment.min_size = 10;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_jitter_exceeds_limit() {
        let mut config = Config::default();
        config.transforms.jitter.max_ms = 1000;
        config.limits.max_jitter_ms = 500;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_valid_rule() {
        let rule = Rule {
            name: "test-rule".to_string(),
            enabled: true,
            priority: 10,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![443]),
                protocols: Some(vec![Protocol::Tcp]),
                ..Default::default()
            },
            transforms: vec![TransformType::Fragment, TransformType::Padding],
            overrides: HashMap::new(),
        };
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn test_parse_json_config() {
        let json = r#"
        {
            "global": {
                "enabled": true,
                "enable_fragmentation": true
            },
            "rules": [
                {
                    "name": "https-evasion",
                    "match_criteria": {
                        "dst_ports": [443],
                        "protocols": ["tcp"]
                    },
                    "transforms": ["fragment", "padding"]
                }
            ],
            "limits": {
                "max_flows": 5000
            }
        }
        "#;
        
        let config = Config::from_json(json).unwrap();
        assert!(config.global.enabled);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.limits.max_flows, 5000);
    }

    #[test]
    fn test_parse_toml_config() {
        let toml_str = r#"
        [global]
        enabled = true
        enable_fragmentation = true

        [[rules]]
        name = "https-evasion"
        transforms = ["fragment", "padding"]

        [rules.match_criteria]
        dst_ports = [443]
        protocols = ["tcp"]

        [limits]
        max_flows = 5000
        "#;
        
        let config = Config::from_toml(toml_str).unwrap();
        assert!(config.global.enabled);
        assert_eq!(config.rules.len(), 1);
    }
}
