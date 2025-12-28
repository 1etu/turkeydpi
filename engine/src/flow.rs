use std::hash::Hash;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::BytesMut;
use lru::LruCache;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::config::{Limits, Protocol, Rule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FlowKey {
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: Protocol,
}

impl FlowKey {
    pub fn new(
        src_ip: IpAddr,
        dst_ip: IpAddr,
        src_port: u16,
        dst_port: u16,
        protocol: Protocol,
    ) -> Self {
        Self {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
        }
    }

    pub fn reverse(&self) -> Self {
        Self {
            src_ip: self.dst_ip,
            dst_ip: self.src_ip,
            src_port: self.dst_port,
            dst_port: self.src_port,
            protocol: self.protocol,
        }
    }

    pub fn is_tcp(&self) -> bool {
        matches!(self.protocol, Protocol::Tcp)
    }

    pub fn is_udp(&self) -> bool {
        matches!(self.protocol, Protocol::Udp)
    }
}

#[derive(Debug)]
pub struct FlowState {
    pub key: FlowKey,
    
    pub created_at: Instant,
    
    pub last_seen: Instant,
    
    pub packet_count: u64,
    
    pub byte_count: u64,
    
    pub matched_rule: Option<String>,
    
    pub direction: FlowDirection,
    
    pub tcp_state: Option<TcpFlowState>,
    
    pub transform_state: TransformState,
}

impl FlowState {
    pub fn new(key: FlowKey) -> Self {
        let now = Instant::now();
        Self {
            key,
            created_at: now,
            last_seen: now,
            packet_count: 0,
            byte_count: 0,
            matched_rule: None,
            direction: FlowDirection::Outbound,
            tcp_state: if key.is_tcp() {
                Some(TcpFlowState::default())
            } else {
                None
            },
            transform_state: TransformState::default(),
        }
    }

    pub fn update(&mut self, size: usize) {
        self.last_seen = Instant::now();
        self.packet_count += 1;
        self.byte_count += size as u64;
    }

    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed() > timeout
    }

    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    pub fn idle_time(&self) -> Duration {
        self.last_seen.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Default)]
pub struct TcpFlowState {
    pub seen_syn: bool,
    
    pub seen_syn_ack: bool,
    
    pub established: bool,
    
    pub seen_fin: bool,
    
    pub closed: bool,
    
    pub initial_seq: Option<u32>,
    
    pub current_seq: Option<u32>,
    
    pub segments_sent: u32,
    
    pub reassembly_bytes: usize,
}

#[derive(Debug, Default)]
pub struct TransformState {
    pub fragment: FragmentState,
    
    pub jitter: JitterState,
    
    pub resegment: ResegmentState,
}

#[derive(Debug, Default)]
pub struct FragmentState {
    pub fragments_generated: u32,
    
    pub pending: Option<BytesMut>,
}

#[derive(Debug, Default)]
pub struct JitterState {
    pub last_jitter_ms: u64,
    
    pub total_jitter_ms: u64,
}

#[derive(Debug, Default)]
pub struct ResegmentState {
    pub buffer: BytesMut,
    
    pub segments_generated: u32,
}

#[derive(Debug)]
pub struct FlowContext<'a> {
    pub key: &'a FlowKey,
    
    pub state: &'a mut FlowState,
    
    pub rule: Option<&'a Rule>,
    
    pub timestamp: Instant,
    
    pub direction: FlowDirection,
    
    pub is_first_packet: bool,
    
    pub output_packets: Vec<BytesMut>,
    
    pub delay: Option<Duration>,
    
    pub drop: bool,
}

impl<'a> FlowContext<'a> {
    pub fn new(
        key: &'a FlowKey,
        state: &'a mut FlowState,
        rule: Option<&'a Rule>,
    ) -> Self {
        let is_first_packet = state.packet_count == 0;
        Self {
            key,
            state,
            rule,
            timestamp: Instant::now(),
            direction: FlowDirection::Outbound,
            is_first_packet,
            output_packets: Vec::with_capacity(4),
            delay: None,
            drop: false,
        }
    }

    pub fn emit(&mut self, packet: BytesMut) {
        self.output_packets.push(packet);
    }

    pub fn request_delay(&mut self, delay: Duration) {
        self.delay = Some(delay);
    }

    pub fn mark_drop(&mut self) {
        self.drop = true;
    }

    pub fn rule_name(&self) -> Option<&str> {
        self.rule.map(|r| r.name.as_str())
    }
}

pub struct FlowCache {
    cache: RwLock<LruCache<FlowKey, FlowState>>,
    max_size: usize,
    timeout: Duration,
    eviction_count: AtomicU64,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
}

impl FlowCache {
    pub fn new(limits: &Limits) -> Self {
        Self {
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(limits.max_flows).unwrap(),
            )),
            max_size: limits.max_flows,
            timeout: Duration::from_secs(limits.flow_timeout_secs),
            eviction_count: AtomicU64::new(0),
            hit_count: AtomicU64::new(0),
            miss_count: AtomicU64::new(0),
        }
    }

    pub fn get_or_create(&self, key: FlowKey) -> FlowState {
        let mut cache = self.cache.write();
        
        if let Some(state) = cache.get_mut(&key) {
            self.hit_count.fetch_add(1, Ordering::Relaxed);
            
            FlowState {
                key: state.key,
                created_at: state.created_at,
                last_seen: state.last_seen,
                packet_count: state.packet_count,
                byte_count: state.byte_count,
                matched_rule: state.matched_rule.clone(),
                direction: state.direction,
                tcp_state: None, 
                transform_state: TransformState::default(),
            }
        } else {
            self.miss_count.fetch_add(1, Ordering::Relaxed);
            
            
            if cache.len() >= self.max_size {
                self.eviction_count.fetch_add(1, Ordering::Relaxed);
            }
            
            let state = FlowState::new(key);
            let result = FlowState::new(key);
            cache.put(key, state);
            result
        }
    }

    pub fn update(&self, state: FlowState) {
        let mut cache = self.cache.write();
        cache.put(state.key, state);
    }

    pub fn cleanup(&self) -> usize {
        let mut cache = self.cache.write();
        let timeout = self.timeout;
        
        let before = cache.len();
        
        
        let expired: Vec<FlowKey> = cache
            .iter()
            .filter(|(_, state)| state.is_expired(timeout))
            .map(|(key, _)| *key)
            .collect();
        
        for key in &expired {
            cache.pop(key);
        }
        
        before - cache.len()
    }

    pub fn stats(&self) -> FlowCacheStats {
        let cache = self.cache.read();
        FlowCacheStats {
            size: cache.len(),
            max_size: self.max_size,
            hit_count: self.hit_count.load(Ordering::Relaxed),
            miss_count: self.miss_count.load(Ordering::Relaxed),
            eviction_count: self.eviction_count.load(Ordering::Relaxed),
        }
    }

    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }

    pub fn clear(&self) {
        self.cache.write().clear();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowCacheStats {
    pub size: usize,
    pub max_size: usize,
    pub hit_count: u64,
    pub miss_count: u64,
    pub eviction_count: u64,
}

impl FlowCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hit_count + self.miss_count;
        if total == 0 {
            0.0
        } else {
            self.hit_count as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn test_key() -> FlowKey {
        FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            12345,
            443,
            Protocol::Tcp,
        )
    }

    #[test]
    fn test_flow_key_reverse() {
        let key = test_key();
        let reversed = key.reverse();
        
        assert_eq!(reversed.src_ip, key.dst_ip);
        assert_eq!(reversed.dst_ip, key.src_ip);
        assert_eq!(reversed.src_port, key.dst_port);
        assert_eq!(reversed.dst_port, key.src_port);
    }

    #[test]
    fn test_flow_state_update() {
        let key = test_key();
        let mut state = FlowState::new(key);
        
        assert_eq!(state.packet_count, 0);
        state.update(100);
        assert_eq!(state.packet_count, 1);
        assert_eq!(state.byte_count, 100);
    }

    #[test]
    fn test_flow_cache_get_or_create() {
        let limits = Limits::default();
        let cache = FlowCache::new(&limits);
        let key = test_key();
        
        
        let _state = cache.get_or_create(key);
        assert_eq!(cache.len(), 1);
        
        
        let _state = cache.get_or_create(key);
        assert_eq!(cache.len(), 1);
        
        let stats = cache.stats();
        assert_eq!(stats.miss_count, 1);
        assert_eq!(stats.hit_count, 1);
    }

    #[test]
    fn test_flow_cache_lru_eviction() {
        let mut limits = Limits::default();
        limits.max_flows = 2;
        let cache = FlowCache::new(&limits);
        
        let key1 = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            1000,
            80,
            Protocol::Tcp,
        );
        let key2 = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4)),
            2000,
            80,
            Protocol::Tcp,
        );
        let key3 = FlowKey::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6)),
            3000,
            80,
            Protocol::Tcp,
        );
        
        cache.get_or_create(key1);
        cache.get_or_create(key2);
        assert_eq!(cache.len(), 2);
        
        
        cache.get_or_create(key3);
        assert_eq!(cache.len(), 2);
        
        let stats = cache.stats();
        assert_eq!(stats.eviction_count, 1);
    }
}
