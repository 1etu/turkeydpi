use std::sync::atomic::{AtomicU64, Ordering};
use serde::{Serialize, Deserialize};

#[derive(Debug, Default)]
pub struct Stats {
    pub packets_in: AtomicU64,
    pub packets_out: AtomicU64,    
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,    
    pub packets_dropped: AtomicU64,    
    pub packets_matched: AtomicU64,    
    pub packets_transformed: AtomicU64,    
    pub transform_errors: AtomicU64,    
    pub active_flows: AtomicU64,    
    pub flows_created: AtomicU64,    
    pub flows_evicted: AtomicU64,    
    pub queue_overflows: AtomicU64,
    pub fragments_generated: AtomicU64,
    pub total_jitter_ms: AtomicU64,
    pub decoys_sent: AtomicU64,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_packet_in(&self, size: usize) {
        self.packets_in.fetch_add(1, Ordering::Relaxed);
        self.bytes_in.fetch_add(size as u64, Ordering::Relaxed);
    }

    pub fn record_packet_out(&self, size: usize) {
        self.packets_out.fetch_add(1, Ordering::Relaxed);
        self.bytes_out.fetch_add(size as u64, Ordering::Relaxed);
    }

    pub fn record_drop(&self) {
        self.packets_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_match(&self) {
        self.packets_matched.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_transform(&self) {
        self.packets_transformed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_transform_error(&self) {
        self.transform_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_flow_created(&self) {
        self.flows_created.fetch_add(1, Ordering::Relaxed);
        self.active_flows.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_flow_evicted(&self) {
        self.flows_evicted.fetch_add(1, Ordering::Relaxed);
        self.active_flows.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn record_queue_overflow(&self) {
        self.queue_overflows.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_fragments(&self, count: u32) {
        self.fragments_generated.fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn record_jitter(&self, ms: u64) {
        self.total_jitter_ms.fetch_add(ms, Ordering::Relaxed);
    }

    pub fn record_decoys(&self, count: u32) {
        self.decoys_sent.fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn set_active_flows(&self, count: usize) {
        self.active_flows.store(count as u64, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            packets_in: self.packets_in.load(Ordering::Relaxed),
            packets_out: self.packets_out.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
            packets_dropped: self.packets_dropped.load(Ordering::Relaxed),
            packets_matched: self.packets_matched.load(Ordering::Relaxed),
            packets_transformed: self.packets_transformed.load(Ordering::Relaxed),
            transform_errors: self.transform_errors.load(Ordering::Relaxed),
            active_flows: self.active_flows.load(Ordering::Relaxed),
            flows_created: self.flows_created.load(Ordering::Relaxed),
            flows_evicted: self.flows_evicted.load(Ordering::Relaxed),
            queue_overflows: self.queue_overflows.load(Ordering::Relaxed),
            fragments_generated: self.fragments_generated.load(Ordering::Relaxed),
            total_jitter_ms: self.total_jitter_ms.load(Ordering::Relaxed),
            decoys_sent: self.decoys_sent.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.packets_in.store(0, Ordering::Relaxed);
        self.packets_out.store(0, Ordering::Relaxed);
        self.bytes_in.store(0, Ordering::Relaxed);
        self.bytes_out.store(0, Ordering::Relaxed);
        self.packets_dropped.store(0, Ordering::Relaxed);
        self.packets_matched.store(0, Ordering::Relaxed);
        self.packets_transformed.store(0, Ordering::Relaxed);
        self.transform_errors.store(0, Ordering::Relaxed);
        self.active_flows.store(0, Ordering::Relaxed);
        self.flows_created.store(0, Ordering::Relaxed);
        self.flows_evicted.store(0, Ordering::Relaxed);
        self.queue_overflows.store(0, Ordering::Relaxed);
        self.fragments_generated.store(0, Ordering::Relaxed);
        self.total_jitter_ms.store(0, Ordering::Relaxed);
        self.decoys_sent.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSnapshot {
    pub packets_in: u64,
    pub packets_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_dropped: u64,
    pub packets_matched: u64,
    pub packets_transformed: u64,
    pub transform_errors: u64,
    pub active_flows: u64,
    pub flows_created: u64,
    pub flows_evicted: u64,
    pub queue_overflows: u64,
    pub fragments_generated: u64,
    pub total_jitter_ms: u64,
    pub decoys_sent: u64,
}

impl StatsSnapshot {
    pub fn packets_per_second(&self, elapsed_secs: f64) -> f64 {
        if elapsed_secs <= 0.0 {
            0.0
        } else {
            self.packets_in as f64 / elapsed_secs
        }
    }

    pub fn bytes_per_second(&self, elapsed_secs: f64) -> f64 {
        if elapsed_secs <= 0.0 {
            0.0
        } else {
            self.bytes_in as f64 / elapsed_secs
        }
    }

    pub fn transform_ratio(&self) -> f64 {
        if self.packets_in == 0 {
            0.0
        } else {
            self.packets_transformed as f64 / self.packets_in as f64
        }
    }

    pub fn drop_ratio(&self) -> f64 {
        if self.packets_in == 0 {
            0.0
        } else {
            self.packets_dropped as f64 / self.packets_in as f64
        }
    }

    pub fn expansion_ratio(&self) -> f64 {
        if self.packets_in == 0 {
            0.0
        } else {
            self.packets_out as f64 / self.packets_in as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_recording() {
        let stats = Stats::new();
        
        stats.record_packet_in(100);
        stats.record_packet_in(200);
        stats.record_packet_out(50);
        stats.record_packet_out(50);
        stats.record_packet_out(50);
        
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.packets_in, 2);
        assert_eq!(snapshot.packets_out, 3);
        assert_eq!(snapshot.bytes_in, 300);
        assert_eq!(snapshot.bytes_out, 150);
    }

    #[test]
    fn test_stats_flow_tracking() {
        let stats = Stats::new();
        
        stats.record_flow_created();
        stats.record_flow_created();
        stats.record_flow_created();
        stats.record_flow_evicted();
        
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.flows_created, 3);
        assert_eq!(snapshot.active_flows, 2);
        assert_eq!(snapshot.flows_evicted, 1);
    }

    #[test]
    fn test_stats_reset() {
        let stats = Stats::new();
        
        stats.record_packet_in(100);
        stats.record_flow_created();
        stats.record_fragments(10);
        
        stats.reset();
        
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.packets_in, 0);
        assert_eq!(snapshot.flows_created, 0);
        assert_eq!(snapshot.fragments_generated, 0);
    }

    #[test]
    fn test_snapshot_ratios() {
        let snapshot = StatsSnapshot {
            packets_in: 100,
            packets_out: 150,
            bytes_in: 10000,
            bytes_out: 15000,
            packets_dropped: 5,
            packets_matched: 80,
            packets_transformed: 75,
            transform_errors: 2,
            active_flows: 10,
            flows_created: 20,
            flows_evicted: 10,
            queue_overflows: 0,
            fragments_generated: 50,
            total_jitter_ms: 1000,
            decoys_sent: 20,
        };
        
        assert_eq!(snapshot.expansion_ratio(), 1.5);
        assert_eq!(snapshot.transform_ratio(), 0.75);
        assert_eq!(snapshot.drop_ratio(), 0.05);
        assert_eq!(snapshot.packets_per_second(10.0), 10.0);
        assert_eq!(snapshot.bytes_per_second(10.0), 1000.0);
    }

    #[test]
    fn test_snapshot_edge_cases() {
        let empty = StatsSnapshot {
            packets_in: 0,
            packets_out: 0,
            bytes_in: 0,
            bytes_out: 0,
            packets_dropped: 0,
            packets_matched: 0,
            packets_transformed: 0,
            transform_errors: 0,
            active_flows: 0,
            flows_created: 0,
            flows_evicted: 0,
            queue_overflows: 0,
            fragments_generated: 0,
            total_jitter_ms: 0,
            decoys_sent: 0,
        };
        
        assert_eq!(empty.expansion_ratio(), 0.0);
        assert_eq!(empty.transform_ratio(), 0.0);
        assert_eq!(empty.drop_ratio(), 0.0);
        assert_eq!(empty.packets_per_second(0.0), 0.0);
    }
}
