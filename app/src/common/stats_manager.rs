use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::cell::UnsafeCell;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;

use crate::common::scoped_task::ScopedTask;

/// Predefined metric names for type safety
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricName {
    /// RPC calls sent to peers
    PeerRpcClientTx,
    /// RPC calls received from peers
    PeerRpcClientRx,
    /// RPC calls sent to peers
    PeerRpcServerTx,
    /// RPC calls received from peers
    PeerRpcServerRx,
    /// RPC call duration in milliseconds
    PeerRpcDuration,
    /// RPC errors
    PeerRpcErrors,

    /// Traffic bytes sent
    TrafficBytesTx,
    /// Traffic bytes received
    TrafficBytesRx,
    /// Traffic bytes forwarded
    TrafficBytesForwarded,
    /// Traffic bytes sent to self
    TrafficBytesSelfTx,
    /// Traffic bytes received from self
    TrafficBytesSelfRx,
    /// Traffic bytes forwarded for foreign network, rx to local
    TrafficBytesForeignForwardRx,
    /// Traffic bytes forwarded for foreign network, tx from local
    TrafficBytesForeignForwardTx,
    /// Traffic bytes forwarded for foreign network, forward
    TrafficBytesForeignForwardForwarded,

    /// Traffic packets sent
    TrafficPacketsTx,
    /// Traffic packets received
    TrafficPacketsRx,
    /// Traffic packets forwarded
    TrafficPacketsForwarded,
    /// Traffic packets sent to self
    TrafficPacketsSelfTx,
    /// Traffic packets received from self
    TrafficPacketsSelfRx,
    /// Traffic packets forwarded for foreign network, rx to local
    TrafficPacketsForeignForwardRx,
    /// Traffic packets forwarded for foreign network, tx from local
    TrafficPacketsForeignForwardTx,
    /// Traffic packets forwarded for foreign network, forward
    TrafficPacketsForeignForwardForwarded,

    /// Compression bytes before compression
    CompressionBytesRxBefore,
    /// Compression bytes after compression
    CompressionBytesRxAfter,
    /// Compression bytes before compression
    CompressionBytesTxBefore,
    /// Compression bytes after compression
    CompressionBytesTxAfter,

    TcpProxyConnect,
}

impl fmt::Display for MetricName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricName::PeerRpcClientTx => write!(f, "peer_rpc_client_tx"),
            MetricName::PeerRpcClientRx => write!(f, "peer_rpc_client_rx"),
            MetricName::PeerRpcServerTx => write!(f, "peer_rpc_server_tx"),
            MetricName::PeerRpcServerRx => write!(f, "peer_rpc_server_rx"),
            MetricName::PeerRpcDuration => write!(f, "peer_rpc_duration_ms"),
            MetricName::PeerRpcErrors => write!(f, "peer_rpc_errors"),

            MetricName::TrafficBytesTx => write!(f, "traffic_bytes_tx"),
            MetricName::TrafficBytesRx => write!(f, "traffic_bytes_rx"),
            MetricName::TrafficBytesForwarded => write!(f, "traffic_bytes_forwarded"),
            MetricName::TrafficBytesSelfTx => write!(f, "traffic_bytes_self_tx"),
            MetricName::TrafficBytesSelfRx => write!(f, "traffic_bytes_self_rx"),
            MetricName::TrafficBytesForeignForwardRx => {
                write!(f, "traffic_bytes_foreign_forward_rx")
            }
            MetricName::TrafficBytesForeignForwardTx => {
                write!(f, "traffic_bytes_foreign_forward_tx")
            }
            MetricName::TrafficBytesForeignForwardForwarded => {
                write!(f, "traffic_bytes_foreign_forward_forwarded")
            }

            MetricName::TrafficPacketsTx => write!(f, "traffic_packets_tx"),
            MetricName::TrafficPacketsRx => write!(f, "traffic_packets_rx"),
            MetricName::TrafficPacketsForwarded => write!(f, "traffic_packets_forwarded"),
            MetricName::TrafficPacketsSelfTx => write!(f, "traffic_packets_self_tx"),
            MetricName::TrafficPacketsSelfRx => write!(f, "traffic_packets_self_rx"),
            MetricName::TrafficPacketsForeignForwardRx => {
                write!(f, "traffic_packets_foreign_forward_rx")
            }
            MetricName::TrafficPacketsForeignForwardTx => {
                write!(f, "traffic_packets_foreign_forward_tx")
            }
            MetricName::TrafficPacketsForeignForwardForwarded => {
                write!(f, "traffic_packets_foreign_forward_forwarded")
            }

            MetricName::CompressionBytesRxBefore => write!(f, "compression_bytes_rx_before"),
            MetricName::CompressionBytesRxAfter => write!(f, "compression_bytes_rx_after"),
            MetricName::CompressionBytesTxBefore => write!(f, "compression_bytes_tx_before"),
            MetricName::CompressionBytesTxAfter => write!(f, "compression_bytes_tx_after"),

            MetricName::TcpProxyConnect => write!(f, "tcp_proxy_connect"),
        }
    }
}

/// Predefined label types for type safety
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LabelType {
    /// Network Name
    NetworkName(String),
    /// Source peer ID
    SrcPeerId(u32),
    /// Destination peer ID
    DstPeerId(u32),
    /// Service name
    ServiceName(String),
    /// Method name
    MethodName(String),
    /// Protocol type
    Protocol(String),
    /// Direction (tx/rx)
    Direction(String),
    /// Compression algorithm
    CompressionAlgo(String),
    /// Error type
    ErrorType(String),
    /// Status
    Status(String),
    /// Dst Ip
    DstIp(String),
    /// Mapped Dst Ip
    MappedDstIp(String),
}

impl fmt::Display for LabelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelType::NetworkName(name) => write!(f, "network_name={}", name),
            LabelType::SrcPeerId(id) => write!(f, "src_peer_id={}", id),
            LabelType::DstPeerId(id) => write!(f, "dst_peer_id={}", id),
            LabelType::ServiceName(name) => write!(f, "service_name={}", name),
            LabelType::MethodName(name) => write!(f, "method_name={}", name),
            LabelType::Protocol(proto) => write!(f, "protocol={}", proto),
            LabelType::Direction(dir) => write!(f, "direction={}", dir),
            LabelType::CompressionAlgo(algo) => write!(f, "compression_algo={}", algo),
            LabelType::ErrorType(err) => write!(f, "error_type={}", err),
            LabelType::Status(status) => write!(f, "status={}", status),
            LabelType::DstIp(ip) => write!(f, "dst_ip={}", ip),
            LabelType::MappedDstIp(ip) => write!(f, "mapped_dst_ip={}", ip),
        }
    }
}

impl LabelType {
    pub fn key(&self) -> &'static str {
        match self {
            LabelType::NetworkName(_) => "network_name",
            LabelType::SrcPeerId(_) => "src_peer_id",
            LabelType::DstPeerId(_) => "dst_peer_id",
            LabelType::ServiceName(_) => "service_name",
            LabelType::MethodName(_) => "method_name",
            LabelType::Protocol(_) => "protocol",
            LabelType::Direction(_) => "direction",
            LabelType::CompressionAlgo(_) => "compression_algo",
            LabelType::ErrorType(_) => "error_type",
            LabelType::Status(_) => "status",
            LabelType::DstIp(_) => "dst_ip",
            LabelType::MappedDstIp(_) => "mapped_dst_ip",
        }
    }

    pub fn value(&self) -> String {
        match self {
            LabelType::NetworkName(name) => name.clone(),
            LabelType::SrcPeerId(id) => id.to_string(),
            LabelType::DstPeerId(id) => id.to_string(),
            LabelType::ServiceName(name) => name.clone(),
            LabelType::MethodName(name) => name.clone(),
            LabelType::Protocol(proto) => proto.clone(),
            LabelType::Direction(dir) => dir.clone(),
            LabelType::CompressionAlgo(algo) => algo.clone(),
            LabelType::ErrorType(err) => err.clone(),
            LabelType::Status(status) => status.clone(),
            LabelType::DstIp(ip) => ip.clone(),
            LabelType::MappedDstIp(ip) => ip.clone(),
        }
    }
}

/// Label represents a key-value pair for metric identification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Label {
    pub key: String,
    pub value: String,
}

impl Label {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    pub fn from_label_type(label_type: &LabelType) -> Self {
        Self {
            key: label_type.key().to_string(),
            value: label_type.value(),
        }
    }
}

/// LabelSet represents a collection of labels for a metric
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LabelSet {
    labels: Vec<Label>,
}

impl LabelSet {
    pub fn new() -> Self {
        Self { labels: Vec::new() }
    }

    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.push(Label::new(key, value));
        self.labels.sort_by(|a, b| a.key.cmp(&b.key)); // Keep labels sorted for consistent hashing
        self
    }

    /// Add a typed label to the set
    pub fn with_label_type(mut self, label_type: LabelType) -> Self {
        self.labels.push(Label::from_label_type(&label_type));
        self.labels.sort_by(|a, b| a.key.cmp(&b.key)); // Keep labels sorted for consistent hashing
        self
    }

    /// Create a LabelSet from multiple LabelTypes
    pub fn from_label_types(label_types: &[LabelType]) -> Self {
        let mut labels = Vec::new();
        for label_type in label_types {
            labels.push(Label::from_label_type(label_type));
        }
        labels.sort_by(|a, b| a.key.cmp(&b.key)); // Keep labels sorted for consistent hashing
        Self { labels }
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Generate a string key for this label set
    pub fn to_key(&self) -> String {
        if self.labels.is_empty() {
            return String::new();
        }

        let mut parts = Vec::with_capacity(self.labels.len());
        for label in &self.labels {
            parts.push(format!("{}={}", label.key, label.value));
        }
        parts.join(",")
    }
}

impl Default for LabelSet {
    fn default() -> Self {
        Self::new()
    }
}

/// UnsafeCounter provides a high-performance counter using UnsafeCell
#[derive(Debug)]
pub struct UnsafeCounter {
    value: UnsafeCell<u64>,
}

impl Default for UnsafeCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl UnsafeCounter {
    pub fn new() -> Self {
        Self {
            value: UnsafeCell::new(0),
        }
    }

    pub fn new_with_value(initial: u64) -> Self {
        Self {
            value: UnsafeCell::new(initial),
        }
    }

    /// Increment the counter by the given amount
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is accessing this counter simultaneously.
    pub unsafe fn add(&self, delta: u64) {
        let ptr = self.value.get();
        *ptr = (*ptr).saturating_add(delta);
    }

    /// Increment the counter by 1
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is accessing this counter simultaneously.
    pub unsafe fn inc(&self) {
        self.add(1);
    }

    /// Get the current value of the counter
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is modifying this counter simultaneously.
    pub unsafe fn get(&self) -> u64 {
        let ptr = self.value.get();
        *ptr
    }

    /// Reset the counter to zero
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is accessing this counter simultaneously.
    pub unsafe fn reset(&self) {
        let ptr = self.value.get();
        *ptr = 0;
    }

    /// Set the counter to a specific value
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is accessing this counter simultaneously.
    pub unsafe fn set(&self, value: u64) {
        let ptr = self.value.get();
        *ptr = value;
    }
}

// UnsafeCounter is Send + Sync because the safety is guaranteed by the caller
unsafe impl Send for UnsafeCounter {}
unsafe impl Sync for UnsafeCounter {}

/// MetricData contains both the counter and last update timestamp
/// Uses UnsafeCell for lock-free access
#[derive(Debug)]
struct MetricData {
    counter: UnsafeCounter,
    last_updated: UnsafeCell<Instant>,
}

impl MetricData {
    fn new() -> Self {
        Self {
            counter: UnsafeCounter::new(),
            last_updated: UnsafeCell::new(Instant::now()),
        }
    }

    fn new_with_value(initial: u64) -> Self {
        Self {
            counter: UnsafeCounter::new_with_value(initial),
            last_updated: UnsafeCell::new(Instant::now()),
        }
    }

    /// Update the last_updated timestamp
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is accessing this timestamp simultaneously.
    unsafe fn touch(&self) {
        let ptr = self.last_updated.get();
        *ptr = Instant::now();
    }

    /// Get the last updated timestamp
    /// # Safety
    /// This method is unsafe because it uses UnsafeCell. The caller must ensure
    /// that no other thread is modifying this timestamp simultaneously.
    unsafe fn get_last_updated(&self) -> Instant {
        let ptr = self.last_updated.get();
        *ptr
    }
}

// MetricData is Send + Sync because the safety is guaranteed by the caller
unsafe impl Send for MetricData {}
unsafe impl Sync for MetricData {}

/// MetricKey uniquely identifies a metric with its name and labels
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MetricKey {
    name: MetricName,
    labels: LabelSet,
}

impl MetricKey {
    fn new(name: MetricName, labels: LabelSet) -> Self {
        Self { name, labels }
    }
}

impl fmt::Display for MetricKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label_str = self.labels.to_key();
        if label_str.is_empty() {
            f.write_str(self.name.to_string().as_str())
        } else {
            f.write_str(format!("{}[{}]", self.name, label_str).as_str())
        }
    }
}

/// CounterHandle provides a safe interface to a MetricData
/// It ensures thread-local access patterns for performance
#[derive(Clone)]
pub struct CounterHandle {
    metric_data: Arc<MetricData>,
    _key: MetricKey, // Keep key for debugging purposes
}

impl CounterHandle {
    fn new(metric_data: Arc<MetricData>, key: MetricKey) -> Self {
        Self {
            metric_data,
            _key: key,
        }
    }

    /// Increment the counter by the given amount
    pub fn add(&self, delta: u64) {
        unsafe {
            self.metric_data.counter.add(delta);
            self.metric_data.touch();
        }
    }

    /// Increment the counter by 1
    pub fn inc(&self) {
        unsafe {
            self.metric_data.counter.inc();
            self.metric_data.touch();
        }
    }

    /// Get the current value of the counter
    pub fn get(&self) -> u64 {
        unsafe { self.metric_data.counter.get() }
    }

    /// Reset the counter to zero
    pub fn reset(&self) {
        unsafe {
            self.metric_data.counter.reset();
            self.metric_data.touch();
        }
    }

    /// Set the counter to a specific value
    pub fn set(&self, value: u64) {
        unsafe {
            self.metric_data.counter.set(value);
            self.metric_data.touch();
        }
    }
}

/// MetricSnapshot represents a point-in-time view of a metric
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub name: MetricName,
    pub labels: LabelSet,
    pub value: u64,
}

impl MetricSnapshot {
    pub fn name_str(&self) -> String {
        self.name.to_string()
    }
}

/// StatsManager manages global statistics with high performance counters
pub struct StatsManager {
    counters: Arc<DashMap<MetricKey, Arc<MetricData>>>,
    cleanup_task: ScopedTask<()>,
}

impl StatsManager {
    /// Create a new StatsManager
    pub fn new() -> Self {
        let counters = Arc::new(DashMap::new());

        // Start cleanup task only if we're in a tokio runtime
        let counters_clone = Arc::downgrade(&counters.clone());
        let cleanup_task = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(60)); // Check every minute
            loop {
                interval.tick().await;

                let Some(cutoff_time) = Instant::now().checked_sub(Duration::from_secs(180)) else {
                    continue;
                };

                let Some(counters) = counters_clone.upgrade() else {
                    break;
                };

                // Remove entries that haven't been updated for 3 minutes
                counters.retain(|_, metric_data: &mut Arc<MetricData>| unsafe {
                    metric_data.get_last_updated() > cutoff_time
                });
            }
        });

        Self {
            counters,
            cleanup_task: cleanup_task.into(),
        }
    }

    /// Get or create a counter with the given name and labels
    pub fn get_counter(&self, name: MetricName, labels: LabelSet) -> CounterHandle {
        let key = MetricKey::new(name, labels);

        let metric_data = self
            .counters
            .entry(key.clone())
            .or_insert_with(|| Arc::new(MetricData::new()))
            .clone();

        CounterHandle::new(metric_data, key)
    }

    /// Get a counter with no labels
    pub fn get_simple_counter(&self, name: MetricName) -> CounterHandle {
        self.get_counter(name, LabelSet::new())
    }

    /// Get all metric snapshots
    pub fn get_all_metrics(&self) -> Vec<MetricSnapshot> {
        let mut metrics = Vec::new();

        for entry in self.counters.iter() {
            let key = entry.key();
            let metric_data = entry.value();

            let value = unsafe { metric_data.counter.get() };

            metrics.push(MetricSnapshot {
                name: key.name,
                labels: key.labels.clone(),
                value,
            });
        }

        // Sort by metric name and then by labels for consistent output
        metrics.sort_by(|a, b| {
            a.name
                .to_string()
                .cmp(&b.name.to_string())
                .then_with(|| a.labels.to_key().cmp(&b.labels.to_key()))
        });

        metrics
    }

    /// Get metrics filtered by name prefix
    pub fn get_metrics_by_prefix(&self, prefix: &str) -> Vec<MetricSnapshot> {
        self.get_all_metrics()
            .into_iter()
            .filter(|m| m.name.to_string().starts_with(prefix))
            .collect()
    }

    /// Get a specific metric by name and labels
    pub fn get_metric(&self, name: MetricName, labels: &LabelSet) -> Option<MetricSnapshot> {
        let key = MetricKey::new(name, labels.clone());

        if let Some(metric_data) = self.counters.get(&key) {
            let value = unsafe { metric_data.counter.get() };
            Some(MetricSnapshot {
                name,
                labels: labels.clone(),
                value,
            })
        } else {
            None
        }
    }

    /// Clear all metrics
    pub fn clear(&self) {
        self.counters.clear();
    }

    /// Get the number of tracked metrics
    pub fn metric_count(&self) -> usize {
        self.counters.len()
    }

    /// Export metrics in Prometheus format
    pub fn export_prometheus(&self) -> String {
        let metrics = self.get_all_metrics();
        let mut output = String::new();

        let mut current_metric = String::new();

        for metric in metrics {
            let metric_name_str = metric.name.to_string();
            if metric_name_str != current_metric {
                if !current_metric.is_empty() {
                    output.push('\n');
                }
                output.push_str(&format!("# TYPE {} counter\n", metric_name_str));
                current_metric = metric_name_str.clone();
            }

            if metric.labels.labels().is_empty() {
                output.push_str(&format!("{} {}\n", metric_name_str, metric.value));
            } else {
                let label_str = metric
                    .labels
                    .labels()
                    .iter()
                    .map(|l| format!("{}=\"{}\"", l.key, l.value))
                    .collect::<Vec<_>>()
                    .join(",");
                output.push_str(&format!(
                    "{}{{{}}} {}\n",
                    metric_name_str, label_str, metric.value
                ));
            }
        }

        output
    }
}

impl Default for StatsManager {
    fn default() -> Self {
        Self::new()
    }
}
