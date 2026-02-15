use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::tls::{find_http_host, fragment_at_offsets, is_client_hello, is_http_request};
use crate::tls::{parse_client_hello, ClientHelloInfo};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BypassConfig {
    pub fragment_sni: bool,

    pub split_offsets: Vec<usize>,

    pub split_at_sni: bool,

    pub sni_split_ratio: f32,

    pub fragment_http_host: bool,

    pub http_split_offset: usize,

    pub fragment_delay_us: u64,

    pub max_segment_size: usize,
}

impl Default for BypassConfig {
    fn default() -> Self {
        Self::aggressive()
    }
}

impl BypassConfig {
    pub fn turk_telekom() -> Self {
        Self {
            fragment_sni: true,
            split_offsets: vec![2],
            split_at_sni: true,
            sni_split_ratio: 0.5,
            fragment_http_host: true,
            http_split_offset: 2,
            fragment_delay_us: 0,
            max_segment_size: 20,
        }
    }

    pub fn vodafone_tr() -> Self {
        Self {
            fragment_sni: true,
            split_offsets: vec![3],
            split_at_sni: true,
            sni_split_ratio: 0.5,
            fragment_http_host: true,
            http_split_offset: 3,
            fragment_delay_us: 100,
            max_segment_size: 30,
        }
    }

    pub fn superonline() -> Self {
        Self {
            fragment_sni: true,
            split_offsets: vec![1],
            split_at_sni: true,
            sni_split_ratio: 0.5,
            fragment_http_host: true,
            http_split_offset: 1,
            fragment_delay_us: 0,
            max_segment_size: 15,
        }
    }

    pub fn aggressive() -> Self {
        Self {
            fragment_sni: true,
            split_offsets: vec![1, 3],
            split_at_sni: true,
            sni_split_ratio: 0.4,
            fragment_http_host: true,
            http_split_offset: 1,
            fragment_delay_us: 10_000,
            max_segment_size: 5,
        }
    }

    pub fn passthrough() -> Self {
        Self {
            fragment_sni: false,
            split_offsets: Vec::new(),
            split_at_sni: false,
            sni_split_ratio: 0.5,
            fragment_http_host: false,
            http_split_offset: 0,
            fragment_delay_us: 0,
            max_segment_size: 0,
        }
    }

    pub fn preset(name: &str) -> Option<Self> {
        match name {
            "turk-telekom" => Some(Self::turk_telekom()),
            "vodafone" => Some(Self::vodafone_tr()),
            "superonline" => Some(Self::superonline()),
            "aggressive" => Some(Self::aggressive()),
            "none" => Some(Self::passthrough()),
            _ => None,
        }
    }

    pub fn preset_names() -> &'static [&'static str] {
        &["turk-telekom", "vodafone", "superonline", "aggressive"]
    }
}

#[derive(Debug, Default)]
pub struct BypassResult {
    pub fragments: Vec<Bytes>,
    pub inter_fragment_delay: Option<Duration>,
    pub modified: bool,
    pub protocol: DetectedProtocol,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetectedProtocol {
    TlsClientHello,
    HttpRequest,
    #[default]
    Unknown,
}

pub struct BypassEngine {
    config: BypassConfig,
}

impl BypassEngine {
    pub fn new(config: BypassConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &BypassConfig {
        &self.config
    }

    pub fn process_outgoing(&self, data: &[u8]) -> BypassResult {
        let mut result = BypassResult::default();

        if is_client_hello(data) {
            result.protocol = DetectedProtocol::TlsClientHello;
            self.process_tls_client_hello(data, &mut result);
        } else if is_http_request(data) {
            result.protocol = DetectedProtocol::HttpRequest;
            self.process_http_request(data, &mut result);
        } else {
            result.fragments.push(Bytes::copy_from_slice(data));
        }

        result
    }

    fn sni_split_point(&self, info: &ClientHelloInfo, len: usize) -> Option<usize> {
        if !self.config.split_at_sni {
            return None;
        }

        let offset = info.sni_offset?;
        let sni_len = info.sni_length?;

        if sni_len < 2 {
            return None;
        }

        let ratio = self.config.sni_split_ratio.clamp(0.0, 1.0);
        let inner = ((sni_len as f32 * ratio) as usize).clamp(1, sni_len - 1);
        let point = offset + inner;

        if point > 0 && point < len {
            Some(point)
        } else {
            None
        }
    }

    fn collect_split_points(&self, info: &ClientHelloInfo, len: usize) -> Vec<usize> {
        let mut points: Vec<usize> = self
            .config
            .split_offsets
            .iter()
            .copied()
            .filter(|&o| o > 0 && o < len)
            .collect();

        if let Some(point) = self.sni_split_point(info, len) {
            points.push(point);
        }

        points.sort_unstable();
        points.dedup();
        points
    }

    fn process_tls_client_hello(&self, data: &[u8], result: &mut BypassResult) {
        if !self.config.fragment_sni {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }

        let info = match parse_client_hello(data) {
            Some(info) => info,
            None => {
                result.fragments.push(Bytes::copy_from_slice(data));
                return;
            }
        };

        result.hostname = info.sni_hostname.clone();

        let points = self.collect_split_points(&info, data.len());
        if points.is_empty() {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }

        let head_limit = points[0];
        let segment_size = self.config.max_segment_size;

        if segment_size > 0 && segment_size < head_limit {
            let mut pos = 0;
            while pos < head_limit {
                let end = (pos + segment_size).min(head_limit);
                result
                    .fragments
                    .push(Bytes::copy_from_slice(&data[pos..end]));
                pos = end;
            }
        } else {
            result
                .fragments
                .push(Bytes::copy_from_slice(&data[..head_limit]));
        }

        let mut prev = head_limit;
        for point in points.iter().skip(1) {
            result
                .fragments
                .push(Bytes::copy_from_slice(&data[prev..*point]));
            prev = *point;
        }
        result.fragments.push(Bytes::copy_from_slice(&data[prev..]));

        result.modified = true;
        self.apply_delay(result);
    }

    fn process_http_request(&self, data: &[u8], result: &mut BypassResult) {
        if !self.config.fragment_http_host {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }

        let (host_offset, host_len) = match find_http_host(data) {
            Some(found) => found,
            None => {
                result.fragments.push(Bytes::copy_from_slice(data));
                return;
            }
        };

        result.hostname = std::str::from_utf8(&data[host_offset..host_offset + host_len])
            .ok()
            .map(|s| s.to_string());

        let offset = self.config.http_split_offset.clamp(1, host_len.max(1));
        let split_pos = host_offset + offset;

        if split_pos == 0 || split_pos >= data.len() {
            result.fragments.push(Bytes::copy_from_slice(data));
            return;
        }

        for fragment in fragment_at_offsets(data, &[split_pos]) {
            result.fragments.push(fragment.freeze());
        }

        result.modified = true;
        self.apply_delay(result);
    }

    fn apply_delay(&self, result: &mut BypassResult) {
        if self.config.fragment_delay_us > 0 {
            result.inter_fragment_delay =
                Some(Duration::from_micros(self.config.fragment_delay_us));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tls_client_hello() -> Vec<u8> {
        vec![
            0x16, 0x03, 0x01, 0x00, 0x5a, 0x01, 0x00, 0x00, 0x56, 0x03, 0x03, 0x00, 0x01, 0x02,
            0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
            0x1f, 0x00, 0x00, 0x02, 0x13, 0x01, 0x01, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00, 0x10,
            0x00, 0x0e, 0x00, 0x00, 0x0b, 0x64, 0x69, 0x73, 0x63, 0x6f, 0x72, 0x64, 0x2e, 0x63,
            0x6f, 0x6d, 0x00, 0x15, 0x00, 0x03, 0x00, 0x00, 0x00,
        ]
    }

    fn reassemble(result: &BypassResult) -> Vec<u8> {
        result
            .fragments
            .iter()
            .flat_map(|f| f.iter().copied())
            .collect()
    }

    #[test]
    fn test_bypass_tls() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = sample_tls_client_hello();

        let result = engine.process_outgoing(&data);

        assert!(result.modified);
        assert_eq!(result.protocol, DetectedProtocol::TlsClientHello);
        assert!(result.fragments.len() >= 2);
        assert_eq!(result.hostname.as_deref(), Some("discord.com"));
        assert_eq!(reassemble(&result), data);
    }

    #[test]
    fn test_bypass_http() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = b"GET / HTTP/1.1\r\nHost: discord.com\r\nConnection: close\r\n\r\n";

        let result = engine.process_outgoing(data);

        assert!(result.modified);
        assert_eq!(result.protocol, DetectedProtocol::HttpRequest);
        assert!(result.fragments.len() >= 2);
        assert_eq!(result.hostname.as_deref(), Some("discord.com"));
        assert_eq!(&reassemble(&result)[..], &data[..]);
    }

    #[test]
    fn test_every_preset_splits_inside_sni() {
        let data = sample_tls_client_hello();
        let info = parse_client_hello(&data).unwrap();
        let sni_start = info.sni_offset.unwrap();
        let sni_end = sni_start + info.sni_length.unwrap();

        for name in BypassConfig::preset_names() {
            let config = BypassConfig::preset(name).unwrap();
            let engine = BypassEngine::new(config);
            let result = engine.process_outgoing(&data);

            assert!(result.modified, "{} did not modify", name);
            assert_eq!(reassemble(&result), data, "{} altered the stream", name);

            let mut boundary = 0;
            let mut split_inside_sni = false;
            for fragment in &result.fragments {
                boundary += fragment.len();
                if boundary > sni_start && boundary < sni_end {
                    split_inside_sni = true;
                }
            }

            assert!(split_inside_sni, "{} left the sni intact", name);
        }
    }

    #[test]
    fn test_unknown_protocol_passthrough() {
        let engine = BypassEngine::new(BypassConfig::default());
        let data = b"some random binary data\x00\x01\x02";

        let result = engine.process_outgoing(data);

        assert!(!result.modified);
        assert_eq!(result.protocol, DetectedProtocol::Unknown);
        assert_eq!(result.fragments.len(), 1);
        assert_eq!(&result.fragments[0][..], &data[..]);
    }

    #[test]
    fn test_passthrough_preset_does_nothing() {
        let engine = BypassEngine::new(BypassConfig::passthrough());
        let data = sample_tls_client_hello();

        let result = engine.process_outgoing(&data);

        assert!(!result.modified);
        assert_eq!(result.fragments.len(), 1);
    }
}
