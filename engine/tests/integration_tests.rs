use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use bytes::BytesMut;

use engine::config::*;
use engine::flow::FlowKey;
use engine::pipeline::Pipeline;
use engine::stats::Stats;
use engine::Config;
use std::sync::Arc;

fn test_config_with_fragmentation() -> Config {
    Config {
        global: GlobalConfig {
            enabled: true,
            enable_fragmentation: true,
            enable_jitter: false,
            enable_padding: false,
            enable_header_normalization: false,
            log_level: "debug".to_string(),
            json_logging: false,
        },
        rules: vec![Rule {
            name: "test-fragment".to_string(),
            enabled: true,
            priority: 100,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![443]),
                protocols: Some(vec![Protocol::Tcp]),
                ..Default::default()
            },
            transforms: vec![TransformType::Fragment],
            overrides: HashMap::new(),
        }],
        limits: Limits::default(),
        transforms: TransformParams {
            fragment: FragmentParams {
                min_size: 1,
                max_size: 10,
                split_at_offset: None,
                randomize: false,
            },
            ..Default::default()
        },
    }
}

fn test_config_multi_transform() -> Config {
    Config {
        global: GlobalConfig {
            enabled: true,
            enable_fragmentation: true,
            enable_jitter: false,
            enable_padding: true,
            enable_header_normalization: false,
            log_level: "debug".to_string(),
            json_logging: false,
        },
        rules: vec![Rule {
            name: "test-multi".to_string(),
            enabled: true,
            priority: 100,
            match_criteria: MatchCriteria {
                dst_ports: Some(vec![443]),
                protocols: Some(vec![Protocol::Tcp]),
                ..Default::default()
            },
            transforms: vec![TransformType::Fragment, TransformType::Padding],
            overrides: HashMap::new(),
        }],
        limits: Limits::default(),
        transforms: TransformParams {
            fragment: FragmentParams {
                min_size: 5,
                max_size: 20,
                split_at_offset: None,
                randomize: false,
            },
            padding: PaddingParams {
                min_bytes: 10,
                max_bytes: 10,
                fill_byte: Some(0xAA),
            },
            ..Default::default()
        },
    }
}

fn https_flow_key() -> FlowKey {
    FlowKey::new(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        54321,
        443,
        Protocol::Tcp,
    )
}

fn http_flow_key() -> FlowKey {
    FlowKey::new(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        54322,
        80,
        Protocol::Tcp,
    )
}

#[test]
fn test_pipeline_processes_matching_traffic() {
    let config = test_config_with_fragmentation();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats.clone()).unwrap();

    let key = https_flow_key();
    let data = BytesMut::from(&b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n"[..]);
    let original_len = data.len();

    let output = pipeline.process(key, data).unwrap();

    assert!(output.matched_rule.is_some());
    assert_eq!(output.matched_rule.as_ref().unwrap(), "test-fragment");

    let all_packets = output.all_packets();
    assert!(all_packets.len() > 1, "Expected multiple fragments");

    let total_len: usize = all_packets.iter().map(|p| p.len()).sum();
    assert_eq!(total_len, original_len);

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.packets_in, 1);
    assert!(snapshot.packets_out >= 1);
    assert_eq!(snapshot.packets_matched, 1);
}

#[test]
fn test_pipeline_passes_through_non_matching_traffic() {
    let config = test_config_with_fragmentation();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats.clone()).unwrap();

    let key = http_flow_key();
    let original = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let data = BytesMut::from(&original[..]);

    let output = pipeline.process(key, data).unwrap();

    assert!(output.matched_rule.is_none());

    assert!(!output.dropped);
    assert!(output.primary.is_some());
    assert_eq!(output.primary.unwrap().as_ref(), original);
    assert!(output.additional.is_empty());
}

#[test]
fn test_pipeline_disabled_passthrough() {
    let mut config = test_config_with_fragmentation();
    config.global.enabled = false;

    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let key = https_flow_key();
    let original = b"test data";
    let data = BytesMut::from(&original[..]);

    let output = pipeline.process(key, data).unwrap();

    assert!(output.primary.is_some());
    assert_eq!(output.primary.unwrap().as_ref(), original);
    assert!(output.additional.is_empty());
}

#[test]
fn test_pipeline_multi_transform() {
    let config = test_config_multi_transform();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let key = https_flow_key();
    let data = BytesMut::from(&b"Hello, this is a test message for multi-transform"[..]);
    let original_len = data.len();

    let output = pipeline.process(key, data).unwrap();

    assert!(output.matched_rule.is_some());

    let all_packets = output.all_packets();
    
    let total_len: usize = all_packets.iter().map(|p| p.len()).sum();
    assert!(
        total_len > original_len,
        "Expected padding to increase size: {} > {}",
        total_len,
        original_len
    );
}

#[test]
fn test_pipeline_config_reload() {
    let config = test_config_with_fragmentation();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let key_443 = https_flow_key();
    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(key_443, data).unwrap();
    assert!(output.matched_rule.is_some());

    let mut new_config = test_config_with_fragmentation();
    new_config.rules[0].match_criteria.dst_ports = Some(vec![8443]);

    pipeline.reload_config(new_config).unwrap();

    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(key_443, data).unwrap();
    assert!(output.matched_rule.is_none());

    let key_8443 = FlowKey::new(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        54321,
        8443,
        Protocol::Tcp,
    );
    let data = BytesMut::from(&b"test data here"[..]);
    let output = pipeline.process(key_8443, data).unwrap();
    assert!(output.matched_rule.is_some());
}

#[test]
fn test_pipeline_flow_tracking() {
    let config = test_config_with_fragmentation();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let key = https_flow_key();

    for i in 0..10 {
        let data = BytesMut::from(format!("Packet number {}", i).as_bytes());
        let _ = pipeline.process(key, data);
    }

    assert!(!pipeline.flow_cache().is_empty());
    
    let cache_stats = pipeline.flow_cache().stats();
    assert!(cache_stats.size >= 1);
}

#[test]
fn test_pipeline_stats_accumulation() {
    let config = test_config_with_fragmentation();
    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats.clone()).unwrap();

    let key = https_flow_key();

    for _ in 0..100 {
        let data = BytesMut::from(&b"test pkt data"[..]);
        let _ = pipeline.process(key, data);
    }

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.packets_in, 100);
    assert!(snapshot.packets_out >= 100);
    assert_eq!(snapshot.packets_matched, 100);
}

#[test]
fn test_multiple_rules_priority() {
    let config = Config {
        global: GlobalConfig {
            enabled: true,
            enable_fragmentation: true,
            enable_jitter: false,
            enable_padding: true,
            enable_header_normalization: false,
            log_level: "debug".to_string(),
            json_logging: false,
        },
        rules: vec![
            Rule {
                name: "catch-all".to_string(),
                enabled: true,
                priority: 0,
                match_criteria: MatchCriteria::default(),
                transforms: vec![TransformType::Padding],
                overrides: HashMap::new(),
            },
            Rule {
                name: "https-specific".to_string(),
                enabled: true,
                priority: 100, 
                match_criteria: MatchCriteria {
                    dst_ports: Some(vec![443]),
                    protocols: Some(vec![Protocol::Tcp]),
                    ..Default::default()
                },
                transforms: vec![TransformType::Fragment],
                overrides: HashMap::new(),
            },
        ],
        limits: Limits::default(),
        transforms: TransformParams::default(),
    };

    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let https_key = https_flow_key();
    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(https_key, data).unwrap();
    assert_eq!(output.matched_rule.unwrap(), "https-specific");

    let http_key = http_flow_key();
    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(http_key, data).unwrap();
    assert_eq!(output.matched_rule.unwrap(), "catch-all");
}

#[test]
fn test_ip_cidr_matching() {
    let config = Config {
        global: GlobalConfig {
            enabled: true,
            enable_fragmentation: false,
            enable_jitter: false,
            enable_padding: true,
            enable_header_normalization: false,
            log_level: "debug".to_string(),
            json_logging: false,
        },
        rules: vec![Rule {
            name: "private-networks".to_string(),
            enabled: true,
            priority: 100,
            match_criteria: MatchCriteria {
                dst_ip: Some(vec![
                    "10.0.0.0/8".to_string(),
                    "172.16.0.0/12".to_string(),
                    "192.168.0.0/16".to_string(),
                ]),
                ..Default::default()
            },
            transforms: vec![TransformType::Padding],
            overrides: HashMap::new(),
        }],
        limits: Limits::default(),
        transforms: TransformParams::default(),
    };

    let stats = Arc::new(Stats::new());
    let pipeline = Pipeline::new(config, stats).unwrap();

    let private_key = FlowKey::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
        1234,
        80,
        Protocol::Tcp,
    );
    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(private_key, data).unwrap();
    assert!(output.matched_rule.is_some());

    let public_key = FlowKey::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        1234,
        80,
        Protocol::Tcp,
    );
    let data = BytesMut::from(&b"test"[..]);
    let output = pipeline.process(public_key, data).unwrap();
    assert!(output.matched_rule.is_none());
}
