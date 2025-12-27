use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use engine::{BypassConfig, BypassEngine, DetectedProtocol, DohResolver};

#[derive(Debug, Default)]
pub struct ProxyStats {
    pub connections_total: AtomicU64,
    pub connections_active: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub bytes_received: AtomicU64,
    pub tls_connections: AtomicU64,
    pub http_connections: AtomicU64,
    pub bypass_applied: AtomicU64,
    pub dns_queries: AtomicU64,
    pub errors: AtomicU64,
}

impl ProxyStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    
    pub fn print_summary(&self) {
        println!("\n📊 Statistics:");
        println!("   Connections: {} total, {} active", 
                 self.connections_total.load(Ordering::Relaxed),
                 self.connections_active.load(Ordering::Relaxed));
        println!("   TLS/HTTPS: {}", self.tls_connections.load(Ordering::Relaxed));
        println!("   HTTP: {}", self.http_connections.load(Ordering::Relaxed));
        println!("   Bypass applied: {}", self.bypass_applied.load(Ordering::Relaxed));
        println!("   DoH DNS queries: {}", self.dns_queries.load(Ordering::Relaxed));
        println!("   Data: {} KB sent, {} KB received",
                 self.bytes_sent.load(Ordering::Relaxed) / 1024,
                 self.bytes_received.load(Ordering::Relaxed) / 1024);
        println!("   Errors: {}", self.errors.load(Ordering::Relaxed));
    }
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub listen_addr: SocketAddr,    
    pub bypass: BypassConfig,    
    pub connect_timeout: Duration,    
    pub buffer_size: usize,    
    pub verbose: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8844".parse().unwrap(),
            bypass: BypassConfig::default(),
            connect_timeout: Duration::from_secs(30),
            buffer_size: 65536,
            verbose: false,
        }
    }
}

pub struct BypassProxy {
    config: ProxyConfig,
    stats: Arc<ProxyStats>,
    dns: Arc<DohResolver>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl BypassProxy {
    pub fn new(config: ProxyConfig) -> Self {
        Self {
            config,
            stats: ProxyStats::new(),
            dns: Arc::new(DohResolver::new()),
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
        }
    }
    
    pub fn stats(&self) -> Arc<ProxyStats> {
        self.stats.clone()
    }
    
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    pub async fn run(&mut self) -> io::Result<()> {
        let listener = TcpListener::bind(self.config.listen_addr).await?;
        let local_addr = listener.local_addr()?;
        
        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║            TurkeyDPI -  Bypass Proxy Started                 ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║  Listening on: {:<46} ║", format!("http://{}", local_addr));
        println!("║  SNI Fragmentation: {:<41} ║", if self.config.bypass.fragment_sni { "ENABLED ✓" } else { "disabled" });
        println!("║  HTTP Host Fragmentation: {:<35} ║", if self.config.bypass.fragment_http_host { "ENABLED ✓" } else { "disabled" });
        println!("║  DNS-over-HTTPS: {:<44} ║", "ENABLED ✓ (bypasses DNS blocking)");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║  Configure your browser HTTP proxy to: {:<21} ║", local_addr);
        println!("║  Press Ctrl+C to stop                                        ║");
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!();
        
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        self.running.store(true, Ordering::SeqCst);
        
        let config = self.config.clone();
        let stats = self.stats.clone();
        let dns = self.dns.clone();
        let running = self.running.clone();
        
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            let config = config.clone();
                            let stats = stats.clone();
                            let dns = dns.clone();
                            
                            stats.connections_total.fetch_add(1, Ordering::Relaxed);
                            stats.connections_active.fetch_add(1, Ordering::Relaxed);
                            
                            let verbose = config.verbose;
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, peer_addr, config, stats.clone(), dns).await {
                                    if verbose {
                                        debug!("Connection error: {}", e);
                                    }
                                    stats.errors.fetch_add(1, Ordering::Relaxed);
                                }
                                stats.connections_active.fetch_sub(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\nShutting down...");
                    break;
                }
            }
        }
        
        running.store(false, Ordering::SeqCst);
        self.stats.print_summary();
        Ok(())
    }
    
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
    }
}

async fn handle_client(
    mut client: TcpStream,
    peer_addr: SocketAddr,
    config: ProxyConfig,
    stats: Arc<ProxyStats>,
    dns: Arc<DohResolver>,
) -> io::Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = client.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    
    let request = String::from_utf8_lossy(&buf[..n]);
    
    
    if request.starts_with("CONNECT ") {
        return handle_connect(client, peer_addr, &request, &buf[..n], config, stats, dns).await;
    }
    
    
    if let Some(target) = extract_http_target(&request) {
        return handle_http_forward(client, peer_addr, &request, &buf[..n], target, config, stats, dns).await;
    }
    
    
    client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nUnsupported request\r\n").await?;
    Ok(())
}

async fn handle_connect(
    mut client: TcpStream,
    peer_addr: SocketAddr,
    request: &str,
    _raw_request: &[u8],
    config: ProxyConfig,
    stats: Arc<ProxyStats>,
    dns: Arc<DohResolver>,
) -> io::Result<()> {
    let target = extract_connect_target(request)?;
    
    if config.verbose {
        debug!("{} -> CONNECT {}", peer_addr, target);
    }
    
    let resolved_addr = match dns.resolve_host_port(&target).await {
        Ok(addr) => {
            stats.dns_queries.fetch_add(1, Ordering::Relaxed);
            if config.verbose {
                debug!("DoH resolved {} -> {}", target, addr);
            }
            addr
        }
        Err(e) => {
            warn!("DoH resolution failed for {}: {}", target, e);
            match tokio::net::lookup_host(&target).await {
                Ok(mut addrs) => {
                    if let Some(addr) = addrs.next() {
                        addr
                    } else {
                        let msg = format!("HTTP/1.1 502 Bad Gateway\r\n\r\nDNS resolution failed: {}\r\n", e);
                        client.write_all(msg.as_bytes()).await?;
                        return Err(io::Error::new(ErrorKind::NotFound, "DNS resolution failed"));
                    }
                }
                Err(_) => {
                    let msg = format!("HTTP/1.1 502 Bad Gateway\r\n\r\nDNS resolution failed: {}\r\n", e);
                    client.write_all(msg.as_bytes()).await?;
                    return Err(io::Error::new(ErrorKind::NotFound, "DNS resolution failed"));
                }
            }
        }
    };
    
    let mut remote = match tokio::time::timeout(
        config.connect_timeout,
        TcpStream::connect(resolved_addr)
    ).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            let msg = format!("HTTP/1.1 502 Bad Gateway\r\n\r\n{}\r\n", e);
            client.write_all(msg.as_bytes()).await?;
            return Err(e);
        }
        Err(_) => {
            client.write_all(b"HTTP/1.1 504 Gateway Timeout\r\n\r\n").await?;
            return Err(io::Error::new(ErrorKind::TimedOut, "Connection timeout"));
        }
    };
    
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
    
    let _ = client.set_nodelay(true);
    let _ = remote.set_nodelay(true);
    
    let mut initial_buf = vec![0u8; config.buffer_size];
    let initial_len = match client.read(&mut initial_buf).await {
        Ok(0) => return Ok(()),
        Ok(n) => n,
        Err(e) => return Err(e),
    };
    
    let engine = BypassEngine::new(config.bypass.clone());
    let result = engine.process_outgoing(&initial_buf[..initial_len]);
    
    match result.protocol {
        DetectedProtocol::TlsClientHello => {
            stats.tls_connections.fetch_add(1, Ordering::Relaxed);
            if let Some(ref host) = result.hostname {
                if result.modified {
                    info!("🔒 {} [SNI fragmented]", host);
                } else if config.verbose {
                    debug!("🔒 {} [passthrough]", host);
                }
            }
        }
        DetectedProtocol::HttpRequest => {
            stats.http_connections.fetch_add(1, Ordering::Relaxed);
            if let Some(ref host) = result.hostname {
                if result.modified {
                    info!("🌐 {} [Host fragmented]", host);
                } else if config.verbose {
                    debug!("🌐 {} [passthrough]", host);
                }
            }
        }
        DetectedProtocol::Unknown => {
            if config.verbose {
                debug!("❓ Unknown protocol to {}", target);
            }
        }
    }
    
    if result.modified {
        stats.bypass_applied.fetch_add(1, Ordering::Relaxed);
    }
    
    for (i, fragment) in result.fragments.iter().enumerate() {
        remote.write_all(fragment).await?;
        stats.bytes_sent.fetch_add(fragment.len() as u64, Ordering::Relaxed);
        
        if i < result.fragments.len() - 1 {
            if let Some(delay) = result.inter_fragment_delay {
                sleep(delay).await;
            }
        }
    }
    remote.flush().await?;
    
    relay_bidirectional(client, remote, stats, config.buffer_size).await;
    
    Ok(())
}

fn extract_connect_target(request: &str) -> io::Result<String> {
    let first_line = request.lines().next().ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidInput, "Empty request")
    })?;
    
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(io::Error::new(ErrorKind::InvalidInput, "Invalid CONNECT"));
    }
    
    let host_port = parts[1];
    
    if host_port.contains(':') {
        Ok(host_port.to_string())
    } else {
        Ok(format!("{}:443", host_port))
    }
}

async fn relay_bidirectional(
    client: TcpStream,
    remote: TcpStream,
    stats: Arc<ProxyStats>,
    buffer_size: usize,
) {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    
    let stats_up = stats.clone();
    let stats_down = stats.clone();
    
    let client_to_remote = async move {
        let mut buf = vec![0u8; buffer_size];
        loop {
            match client_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if remote_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    stats_up.bytes_sent.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
        let _ = remote_write.shutdown().await;
    };
    
    let remote_to_client = async move {
        let mut buf = vec![0u8; buffer_size];
        loop {
            match remote_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if client_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    stats_down.bytes_received.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
        let _ = client_write.shutdown().await;
    };
    
    tokio::join!(client_to_remote, remote_to_client);
}

fn extract_http_target(request: &str) -> Option<String> {
    let first_line = request.lines().next()?;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    
    if parts.len() < 2 {
        return None;
    }
    
    let method = parts[0];
    let url = parts[1];
    
    
    if !["GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH"].contains(&method) {
        return None;
    }
    
    
    if url.starts_with("http://") {
        let without_scheme = &url[7..];
        let host_end = without_scheme.find('/').unwrap_or(without_scheme.len());
        let host_port = &without_scheme[..host_end];
        
        
        if host_port.contains(':') {
            return Some(host_port.to_string());
        } else {
            return Some(format!("{}:80", host_port));
        }
    }
    
    
    for line in request.lines() {
        if line.to_lowercase().starts_with("host:") {
            let host = line[5..].trim();
            if host.contains(':') {
                return Some(host.to_string());
            } else {
                return Some(format!("{}:80", host));
            }
        }
    }
    
    None
}

async fn handle_http_forward(
    mut client: TcpStream,
    peer_addr: SocketAddr,
    request: &str,
    raw_request: &[u8],
    target: String,
    config: ProxyConfig,
    stats: Arc<ProxyStats>,
    dns: Arc<DohResolver>,
) -> io::Result<()> {
    if config.verbose {
        debug!("{} -> HTTP {}", peer_addr, target);
    }
    
    
    let resolved_addr = match dns.resolve_host_port(&target).await {
        Ok(addr) => {
            stats.dns_queries.fetch_add(1, Ordering::Relaxed);
            addr
        }
        Err(_) => {
            match tokio::net::lookup_host(&target).await {
                Ok(mut addrs) => {
                    if let Some(addr) = addrs.next() {
                        addr
                    } else {
                        client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                        return Err(io::Error::new(ErrorKind::NotFound, "DNS resolution failed"));
                    }
                }
                Err(e) => {
                    client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                    return Err(io::Error::new(ErrorKind::NotFound, e.to_string()));
                }
            }
        }
    };
    
    
    let mut remote = match tokio::time::timeout(
        config.connect_timeout,
        TcpStream::connect(resolved_addr)
    ).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            let msg = format!("HTTP/1.1 502 Bad Gateway\r\n\r\n{}\r\n", e);
            client.write_all(msg.as_bytes()).await?;
            return Err(e);
        }
        Err(_) => {
            client.write_all(b"HTTP/1.1 504 Gateway Timeout\r\n\r\n").await?;
            return Err(io::Error::new(ErrorKind::TimedOut, "Connection timeout"));
        }
    };
    
    
    let rewritten_request = rewrite_http_request(request, raw_request);
    
    
    if let Some(host) = extract_host_header(request) {
        info!("🌐 {} [HTTP forwarded]", host);
    }
    
    stats.http_connections.fetch_add(1, Ordering::Relaxed);
    
    
    remote.write_all(&rewritten_request).await?;
    stats.bytes_sent.fetch_add(rewritten_request.len() as u64, Ordering::Relaxed);
    
    
    let (mut client_read, mut client_write) = client.into_split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    
    let stats_clone = stats.clone();
    let buffer_size = config.buffer_size;
    let idle_timeout = std::time::Duration::from_secs(30);
    
    let client_to_remote = async {
        let mut buf = vec![0u8; buffer_size];
        loop {
            match tokio::time::timeout(idle_timeout, client_read.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    if remote_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    stats_clone.bytes_sent.fetch_add(n as u64, Ordering::Relaxed);
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
    };
    
    let stats_clone2 = stats.clone();
    let remote_to_client = async {
        let mut buf = vec![0u8; buffer_size];
        loop {
            match tokio::time::timeout(idle_timeout, remote_read.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    if client_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    stats_clone2.bytes_received.fetch_add(n as u64, Ordering::Relaxed);
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
    };
    
    
    tokio::select! {
        _ = client_to_remote => {},
        _ = remote_to_client => {},
    }
    
    Ok(())
}

fn rewrite_http_request(request: &str, raw: &[u8]) -> Vec<u8> {
    let first_line = match request.lines().next() {
        Some(line) => line,
        None => return raw.to_vec(),
    };
    
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 3 {
        return raw.to_vec();
    }
    
    let method = parts[0];
    let url = parts[1];
    let version = parts[2];
    
    
    let path = if url.starts_with("http://") {
        let without_scheme = &url[7..];
        if let Some(slash_pos) = without_scheme.find('/') {
            &without_scheme[slash_pos..]
        } else {
            "/"
        }
    } else {
        url
    };
    
    
    let new_first_line = format!("{} {} {}", method, path, version);
    
    
    let first_line_end = raw.iter().position(|&b| b == b'\r' || b == b'\n').unwrap_or(0);
    
    
    let mut result = new_first_line.into_bytes();
    result.extend_from_slice(&raw[first_line_end..]);
    result
}

fn extract_host_header(request: &str) -> Option<String> {
    for line in request.lines() {
        if line.to_lowercase().starts_with("host:") {
            return Some(line[5..].trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_connect_target() {
        let req = "CONNECT discord.com:443 HTTP/1.1\r\nHost: discord.com\r\n\r\n";
        assert_eq!(extract_connect_target(req).unwrap(), "discord.com:443");
        
        let req2 = "CONNECT example.com HTTP/1.1\r\n\r\n";
        assert_eq!(extract_connect_target(req2).unwrap(), "example.com:443");
    }
    
    #[test]
    fn test_default_config() {
        let config = ProxyConfig::default();
        assert_eq!(config.listen_addr.port(), 8844);
        assert!(config.bypass.fragment_sni);
        assert!(config.bypass.fragment_http_host);
    }
}
