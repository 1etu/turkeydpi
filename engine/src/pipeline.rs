use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use bytes::BytesMut;
use ipnet::IpNet;
use parking_lot::RwLock;
use tracing::{debug, trace, warn};

use crate::config::{Config, Rule, TransformType};
use crate::error::{EngineError, Result};
use crate::flow::{FlowCache, FlowContext, FlowKey};
use crate::stats::Stats;
use crate::transform::{
    BoxedTransform, TransformResult,
    FragmentTransform, JitterTransform, PaddingTransform,
    HeaderNormalizationTransform, ResegmentTransform, DecoyTransform,
};

#[derive(Debug)]
pub struct PipelineOutput {
    pub primary: Option<BytesMut>,
    pub additional: Vec<BytesMut>,    
    pub delay: Option<std::time::Duration>,    
    pub dropped: bool,    
    pub matched_rule: Option<String>,
}

impl PipelineOutput {
    pub fn dropped() -> Self {
        Self {
            primary: None,
            additional: Vec::new(),
            delay: None,
            dropped: true,
            matched_rule: None,
        }
    }

    pub fn passthrough(data: BytesMut) -> Self {
        Self {
            primary: Some(data),
            additional: Vec::new(),
            delay: None,
            dropped: false,
            matched_rule: None,
        }
    }

    pub fn all_packets(self) -> Vec<BytesMut> {
        let mut packets = Vec::new();
        if let Some(primary) = self.primary {
            packets.push(primary);
        }
        packets.extend(self.additional);
        packets
    }
}

pub struct Pipeline {
    config: RwLock<Arc<Config>>,
    flow_cache: FlowCache,
    stats: Arc<Stats>,    
    transforms: RwLock<HashMap<TransformType, BoxedTransform>>,    
    compiled_rules: RwLock<Vec<CompiledRule>>,
}

struct CompiledRule {
    rule: Rule,    
    dst_nets: Vec<IpNet>,    
    src_nets: Vec<IpNet>,
}

impl CompiledRule {
    fn compile(rule: Rule) -> Result<Self> {
        let dst_nets = match &rule.match_criteria.dst_ip {
            Some(ips) => ips
                .iter()
                .map(|s| {
                    s.parse::<IpNet>()
                        .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                })
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| EngineError::Config(format!("Invalid IP: {}", e)))?,
            None => Vec::new(),
        };
        
        let src_nets = match &rule.match_criteria.src_ip {
            Some(ips) => ips
                .iter()
                .map(|s| {
                    s.parse::<IpNet>()
                        .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                })
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| EngineError::Config(format!("Invalid IP: {}", e)))?,
            None => Vec::new(),
        };
        
        Ok(Self {
            rule,
            dst_nets,
            src_nets,
        })
    }

    fn matches(&self, key: &FlowKey) -> bool {
        let criteria = &self.rule.match_criteria;
        
        if let Some(ref protocols) = criteria.protocols {
            if !protocols.contains(&key.protocol) {
                return false;
            }
        }
        
        if let Some(ref ports) = criteria.dst_ports {
            if !ports.contains(&key.dst_port) {
                return false;
            }
        }
        
        if let Some(ref ports) = criteria.src_ports {
            if !ports.contains(&key.src_port) {
                return false;
            }
        }
        
        if !self.dst_nets.is_empty() {
            let matches_any = self.dst_nets.iter().any(|net| net.contains(&key.dst_ip));
            if !matches_any {
                return false;
            }
        }
        
        if !self.src_nets.is_empty() {
            let matches_any = self.src_nets.iter().any(|net| net.contains(&key.src_ip));
            if !matches_any {
                return false;
            }
        }
        
        true
    }
}

impl Pipeline {
    pub fn new(config: Config, stats: Arc<Stats>) -> Result<Self> {
        config.validate()?;
        
        let flow_cache = FlowCache::new(&config.limits);
        let transforms = Self::create_transforms(&config);
        let compiled_rules = Self::compile_rules(&config.rules)?;
        
        Ok(Self {
            config: RwLock::new(Arc::new(config)),
            flow_cache,
            stats,
            transforms: RwLock::new(transforms),
            compiled_rules: RwLock::new(compiled_rules),
        })
    }

    fn create_transforms(config: &Config) -> HashMap<TransformType, BoxedTransform> {
        let params = &config.transforms;
        let mut transforms: HashMap<TransformType, BoxedTransform> = HashMap::new();
        
        transforms.insert(
            TransformType::Fragment,
            Box::new(FragmentTransform::new(&params.fragment)),
        );
        transforms.insert(
            TransformType::Resegment,
            Box::new(ResegmentTransform::new(&params.resegment)),
        );
        transforms.insert(
            TransformType::Padding,
            Box::new(PaddingTransform::new(&params.padding)),
        );
        transforms.insert(
            TransformType::Jitter,
            Box::new(JitterTransform::new(&params.jitter)),
        );
        transforms.insert(
            TransformType::HeaderNormalization,
            Box::new(HeaderNormalizationTransform::new(&params.header)),
        );
        transforms.insert(
            TransformType::Decoy,
            Box::new(DecoyTransform::new(&params.decoy)),
        );
        
        transforms
    }

    fn compile_rules(rules: &[Rule]) -> Result<Vec<CompiledRule>> {
        let mut compiled: Vec<CompiledRule> = rules
            .iter()
            .filter(|r| r.enabled)
            .cloned()
            .map(CompiledRule::compile)
            .collect::<Result<Vec<_>>>()?;
        
        compiled.sort_by(|a, b| b.rule.priority.cmp(&a.rule.priority));
        
        Ok(compiled)
    }

    pub fn reload_config(&self, new_config: Config) -> Result<()> {
        new_config.validate()?;
        
        let new_transforms = Self::create_transforms(&new_config);
        let new_compiled = Self::compile_rules(&new_config.rules)?;
        
        {
            let mut transforms = self.transforms.write();
            *transforms = new_transforms;
        }
        {
            let mut compiled = self.compiled_rules.write();
            *compiled = new_compiled;
        }
        {
            let mut config = self.config.write();
            *config = Arc::new(new_config);
        }
        
        debug!("Configuration reloaded successfully");
        Ok(())
    }

    pub fn config(&self) -> Arc<Config> {
        self.config.read().clone()
    }

    fn find_matching_rule(&self, key: &FlowKey) -> Option<Rule> {
        let compiled = self.compiled_rules.read();
        
        for compiled_rule in compiled.iter() {
            if compiled_rule.matches(key) {
                trace!(
                    flow = ?key,
                    rule = %compiled_rule.rule.name,
                    "matched rule"
                );
                return Some(compiled_rule.rule.clone());
            }
        }
        
        None
    }

    pub fn process(&self, key: FlowKey, mut data: BytesMut) -> Result<PipelineOutput> {
        let config = self.config.read().clone();
        
        if !config.global.enabled {
            return Ok(PipelineOutput::passthrough(data));
        }
        
        self.stats.record_packet_in(data.len());
        
        let mut flow_state = self.flow_cache.get_or_create(key);
        let is_new_flow = flow_state.packet_count == 0;
        
        if is_new_flow {
            self.stats.record_flow_created();
        }
        
        let matched_rule = self.find_matching_rule(&key);
        
        if matched_rule.is_some() {
            self.stats.record_match();
        }
        
        let rule = match matched_rule {
            Some(r) => r,
            None => {
                flow_state.update(data.len());
                self.flow_cache.update(flow_state);
                return Ok(PipelineOutput::passthrough(data));
            }
        };
        
        let rule_ref = &rule;
        let mut ctx = FlowContext::new(&key, &mut flow_state, Some(rule_ref));
        
        let transforms = self.transforms.read();
        
        for transform_type in &rule.transforms {
            let enabled = match transform_type {
                TransformType::Fragment => config.global.enable_fragmentation,
                TransformType::Jitter => config.global.enable_jitter,
                TransformType::Padding => config.global.enable_padding,
                TransformType::HeaderNormalization => config.global.enable_header_normalization,
                _ => true,
            };
            
            if !enabled {
                continue;
            }
            
            let transform = match transforms.get(transform_type) {
                Some(t) => t,
                None => {
                    warn!(transform = ?transform_type, "transform not found");
                    continue;
                }
            };
            
            trace!(
                transform = transform.name(),
                flow = ?key,
                "applying transform"
            );
            
            let result = match transform.apply(&mut ctx, &mut data) {
                Ok(r) => r,
                Err(e) => {
                    self.stats.record_transform_error();
                    warn!(
                        transform = transform.name(),
                        error = %e,
                        "transform error"
                    );
                    continue;
                }
            };
            
            match result {
                TransformResult::Continue => {}
                TransformResult::Fragmented => {
                    self.stats.record_transform();
                    let fragment_count = ctx.output_packets.len() + 1;
                    self.stats.record_fragments(fragment_count as u32);
                }
                TransformResult::Delay => {
                    self.stats.record_transform();
                    if let Some(delay) = ctx.delay {
                        self.stats.record_jitter(delay.as_millis() as u64);
                    }
                }
                TransformResult::Drop => {
                    ctx.mark_drop();
                    break;
                }
                TransformResult::Skip => {
                    break;
                }
                TransformResult::Error(msg) => {
                    self.stats.record_transform_error();
                    warn!(transform = transform.name(), error = %msg, "transform error");
                }
            }
        }
        
        ctx.state.update(data.len());
        ctx.state.matched_rule = Some(rule.name.clone());
        
        let should_drop = ctx.drop;
        let output_packets = std::mem::take(&mut ctx.output_packets);
        let delay = ctx.delay;
        
        drop(transforms);
        drop(ctx);
        
        self.flow_cache.update(flow_state);
        
        if should_drop {
            self.stats.record_drop();
            return Ok(PipelineOutput::dropped());
        }
        
        self.stats.record_packet_out(data.len());
        for packet in &output_packets {
            self.stats.record_packet_out(packet.len());
        }
        
        Ok(PipelineOutput {
            primary: Some(data),
            additional: output_packets,
            delay,
            dropped: false,
            matched_rule: Some(rule.name),
        })
    }

    pub fn flow_cache(&self) -> &FlowCache {
        &self.flow_cache
    }

    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }

    pub fn cleanup(&self) -> usize {
        let evicted = self.flow_cache.cleanup();
        for _ in 0..evicted {
            self.stats.record_flow_evicted();
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use crate::config::{MatchCriteria, Protocol};

    fn test_config() -> Config {
        let mut config = Config::default();
        config.rules.push(Rule {
            name: "test-https".to_string(),
            enabled: true,
            priority: 10,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![443]),
                protocols: Some(vec![Protocol::Tcp]),
                ..Default::default()
            },
            transforms: vec![TransformType::Fragment, TransformType::Padding],
            overrides: HashMap::new(),
        });
        config
    }

    fn test_flow_key(dst_port: u16) -> FlowKey {
        FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            12345,
            dst_port,
            Protocol::Tcp,
        )
    }

    #[test]
    fn test_pipeline_creation() {
        let config = test_config();
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats);
        assert!(pipeline.is_ok());
    }

    #[test]
    fn test_pipeline_rule_matching() {
        let config = test_config();
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats).unwrap();
        
        let key_443 = test_flow_key(443);
        let rule = pipeline.find_matching_rule(&key_443);
        assert!(rule.is_some());
        assert_eq!(rule.unwrap().name, "test-https");
        
        let key_80 = test_flow_key(80);
        let rule = pipeline.find_matching_rule(&key_80);
        assert!(rule.is_none());
    }

    #[test]
    fn test_pipeline_passthrough() {
        let mut config = Config::default();
        config.global.enabled = true;
        
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats.clone()).unwrap();
        
        let key = test_flow_key(80);
        let data = BytesMut::from(&b"test data"[..]);
        
        let output = pipeline.process(key, data.clone()).unwrap();
        
        assert!(!output.dropped);
        assert!(output.primary.is_some());
        assert_eq!(output.primary.unwrap(), data);
        assert!(output.additional.is_empty());
    }

    #[test]
    fn test_pipeline_disabled() {
        let mut config = Config::default();
        config.global.enabled = false;
        
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats).unwrap();
        
        let key = test_flow_key(443);
        let data = BytesMut::from(&b"test data"[..]);
        
        let output = pipeline.process(key, data.clone()).unwrap();
        
        assert!(!output.dropped);
        assert!(output.primary.is_some());
        assert_eq!(output.primary.unwrap(), data);
    }

    #[test]
    fn test_pipeline_transform_application() {
        let config = test_config();
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats.clone()).unwrap();
        
        let key = test_flow_key(443);
        let data = BytesMut::from(&b"This is a longer test message for fragmentation testing"[..]);
        let original_len = data.len();
        
        let output = pipeline.process(key, data).unwrap();
        
        assert!(!output.dropped);
        assert!(output.matched_rule.is_some());
        
        let total_len: usize = output.all_packets().iter().map(|p| p.len()).sum();
        assert!(total_len >= original_len); 
    }

    #[test]
    fn test_pipeline_stats_tracking() {
        let config = test_config();
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats.clone()).unwrap();
        
        let key = test_flow_key(443);
        let data = BytesMut::from(&b"test data"[..]);
        
        let _ = pipeline.process(key, data);
        
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.packets_in, 1);
        assert!(snapshot.packets_out >= 1);
        assert_eq!(snapshot.packets_matched, 1);
    }

    #[test]
    fn test_pipeline_config_reload() {
        let config = test_config();
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats).unwrap();
        
        let mut new_config = Config::default();
        new_config.rules.push(Rule {
            name: "new-rule".to_string(),
            enabled: true,
            priority: 20,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![8080]),
                ..Default::default()
            },
            transforms: vec![TransformType::Padding],
            overrides: HashMap::new(),
        });
        
        assert!(pipeline.reload_config(new_config).is_ok());
        
        let key = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            12345,
            8080,
            Protocol::Tcp,
        );
        let rule = pipeline.find_matching_rule(&key);
        assert!(rule.is_some());
        assert_eq!(rule.unwrap().name, "new-rule");
    }

    #[test]
    fn test_rule_priority() {
        let mut config = Config::default();
        
        config.rules.push(Rule {
            name: "catch-all".to_string(),
            enabled: true,
            priority: 0,
            match_criteria: MatchCriteria::default(),
            transforms: vec![TransformType::Padding],
            overrides: HashMap::new(),
        });
        
        config.rules.push(Rule {
            name: "specific".to_string(),
            enabled: true,
            priority: 100,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![443]),
                ..Default::default()
            },
            transforms: vec![TransformType::Fragment],
            overrides: HashMap::new(),
        });
        
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats).unwrap();
        
        let key = test_flow_key(443);
        let rule = pipeline.find_matching_rule(&key);
        assert!(rule.is_some());
        assert_eq!(rule.unwrap().name, "specific");
    }

    #[test]
    fn test_ip_matching() {
        let mut config = Config::default();
        config.rules.push(Rule {
            name: "google-dns".to_string(),
            enabled: true,
            priority: 10,
            match_criteria: MatchCriteria {
                dst_ip: Some(vec!["8.8.8.0/24".to_string()]),
                ..Default::default()
            },
            transforms: vec![TransformType::Padding],
            overrides: HashMap::new(),
        });
        
        let stats = Arc::new(Stats::new());
        let pipeline = Pipeline::new(config, stats).unwrap();
        
        let key1 = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            12345,
            53,
            Protocol::Udp,
        );
        assert!(pipeline.find_matching_rule(&key1).is_some());
        
        let key2 = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            12345,
            53,
            Protocol::Udp,
        );
        assert!(pipeline.find_matching_rule(&key2).is_none());
    }
}
