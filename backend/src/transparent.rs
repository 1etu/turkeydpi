use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use engine::{BypassConfig, BypassEngine, BypassResult, DetectedProtocol, DohResolver, DomainList};

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
        println!("\nStatistics:");
        println!(
            "   Connections: {} total, {} active",
            self.connections_total.load(Ordering::Relaxed),
            self.connections_active.load(Ordering::Relaxed)
        );
        println!(
            "   TLS/HTTPS: {}",
            self.tls_connections.load(Ordering::Relaxed)
        );
        println!("   HTTP: {}", self.http_connections.load(Ordering::Relaxed));
        println!(
            "   Bypass applied: {}",
            self.bypass_applied.load(Ordering::Relaxed)
        );
        println!(
            "   DoH DNS queries: {}",
            self.dns_queries.load(Ordering::Relaxed)
        );
        println!(
            "   Data: {} KB sent, {} KB received",
            self.bytes_sent.load(Ordering::Relaxed) / 1024,
            self.bytes_received.load(Ordering::Relaxed) / 1024
        );
        println!("   Errors: {}", self.errors.load(Ordering::Relaxed));
    }
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub listen_addr: SocketAddr,
    pub bypass: BypassConfig,
    pub connect_timeout: Duration,
    pub buffer_size: usize,
    pub max_connections: usize,
    pub allow_system_dns: bool,
    pub domains: Option<Arc<DomainList>>,
    pub verbose: bool,
    pub quiet: bool,
}

impl ProxyConfig {
    fn bypass_for(&self, host: &str) -> BypassConfig {
        match &self.domains {
            Some(list) if !list.matches(host) => BypassConfig::passthrough(),
            _ => self.bypass.clone(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8844".parse().unwrap(),
            bypass: BypassConfig::default(),
            connect_timeout: Duration::from_secs(30),
            buffer_size: 32768,
            max_connections: 512,
            allow_system_dns: false,
            domains: None,
            verbose: false,
            quiet: false,
        }
    }
}

#[derive(Clone)]
struct Session {
    config: ProxyConfig,
    stats: Arc<ProxyStats>,
    dns: Arc<DohResolver>,
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

        if self.config.quiet {
            info!(addr = %local_addr, "bypass proxy listening");
        } else {
            let on_off = |enabled: bool| if enabled { "on" } else { "off" };

            println!("TurkeyDPI bypass proxy");
            println!("  listening      http://{}", local_addr);
            println!(
                "  sni split      {}",
                on_off(self.config.bypass.fragment_sni)
            );
            println!(
                "  host split     {}",
                on_off(self.config.bypass.fragment_http_host)
            );
            println!("  dns-over-https on");
            println!("  max clients    {}", self.config.max_connections);
            println!();
            println!("Set the system or browser HTTP proxy to {}", local_addr);
            println!("Press Ctrl+C to stop");
            println!();
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        self.running.store(true, Ordering::SeqCst);

        let config = self.config.clone();
        let stats = self.stats.clone();
        let dns = self.dns.clone();
        let running = self.running.clone();
        let limit = Arc::new(Semaphore::new(self.config.max_connections.max(1)));

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            let permit = match limit.clone().try_acquire_owned() {
                                Ok(permit) => permit,
                                Err(_) => {
                                    warn!("connection limit reached, rejecting {}", peer_addr);
                                    continue;
                                }
                            };

                            let session = Session {
                                config: config.clone(),
                                stats: stats.clone(),
                                dns: dns.clone(),
                            };

                            stats.connections_total.fetch_add(1, Ordering::Relaxed);
                            stats.connections_active.fetch_add(1, Ordering::Relaxed);

                            let verbose = config.verbose;
                            let stats = stats.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, peer_addr, session).await {
                                    if verbose {
                                        debug!("Connection error: {}", e);
                                    }
                                    stats.errors.fetch_add(1, Ordering::Relaxed);
                                }
                                stats.connections_active.fetch_sub(1, Ordering::Relaxed);
                                drop(permit);
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
        if !self.config.quiet {
            self.stats.print_summary();
        }
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
    session: Session,
) -> io::Result<()> {
    let buf = match read_request_head(&mut client).await? {
        Some(buf) => buf,
        None => return Ok(()),
    };

    let request = String::from_utf8_lossy(&buf);

    if request.starts_with("CONNECT ") {
        return handle_connect(client, peer_addr, &request, &session).await;
    }

    if let Some(target) = extract_http_target(&request) {
        return handle_http_forward(client, peer_addr, &request, &buf, target, &session).await;
    }

    client
        .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nUnsupported request\r\n")
        .await?;
    Ok(())
}

async fn handle_connect(
    mut client: TcpStream,
    peer_addr: SocketAddr,
    request: &str,
    session: &Session,
) -> io::Result<()> {
    let config = &session.config;
    let stats = &session.stats;
    let target = extract_connect_target(request)?;

    if config.verbose {
        debug!("{} -> CONNECT {}", peer_addr, target);
    }

    let mut remote = match connect_target(&target, session).await {
        Ok(stream) => stream,
        Err(e) => {
            let status = if e.kind() == ErrorKind::TimedOut {
                "504 Gateway Timeout"
            } else {
                "502 Bad Gateway"
            };
            let msg = format!("HTTP/1.1 {}\r\n\r\n{}\r\n", status, e);
            client.write_all(msg.as_bytes()).await?;
            return Err(e);
        }
    };

    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    let _ = client.set_nodelay(true);
    let _ = remote.set_nodelay(true);

    let mut initial_buf = vec![0u8; config.buffer_size];
    let initial_len = match client.read(&mut initial_buf).await {
        Ok(0) => return Ok(()),
        Ok(n) => n,
        Err(e) => return Err(e),
    };

    let host = target.split(':').next().unwrap_or(&target);
    let engine = BypassEngine::new(config.bypass_for(host));
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

    write_fragments(&mut remote, &result, stats).await?;

    relay_bidirectional(client, remote, stats.clone()).await;

    Ok(())
}

async fn connect_target(target: &str, session: &Session) -> io::Result<TcpStream> {
    let config = &session.config;
    let stats = &session.stats;
    let dns = &session.dns;
    let addrs = match dns.resolve_socket_addrs(target).await {
        Ok(addrs) => {
            stats.dns_queries.fetch_add(1, Ordering::Relaxed);
            if config.verbose {
                debug!("DoH resolved {} -> {:?}", target, addrs);
            }
            addrs
        }
        Err(e) => {
            if !config.allow_system_dns {
                warn!("DoH resolution failed for {}: {}", target, e);
                return Err(e);
            }

            warn!(
                "DoH failed for {}, falling back to system dns: {}",
                target, e
            );
            tokio::net::lookup_host(target).await?.collect()
        }
    };

    let mut last_error = None;

    for addr in addrs {
        match tokio::time::timeout(config.connect_timeout, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => {
                if config.verbose {
                    debug!("connect to {} failed: {}", addr, e);
                }
                last_error = Some(e);
            }
            Err(_) => {
                last_error = Some(io::Error::new(ErrorKind::TimedOut, "connect timeout"));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::new(ErrorKind::NotFound, "no address to connect")))
}

const MAX_HEADER_BYTES: usize = 64 * 1024;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(15);

async fn read_request_head(client: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    loop {
        let n = match tokio::time::timeout(HEADER_READ_TIMEOUT, client.read(&mut chunk)).await {
            Ok(Ok(0)) => {
                if buf.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(buf));
            }
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(io::Error::new(ErrorKind::TimedOut, "header read timeout")),
        };

        buf.extend_from_slice(&chunk[..n]);

        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(Some(buf));
        }

        if buf.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "request head too big",
            ));
        }
    }
}

async fn write_fragments(
    remote: &mut TcpStream,
    result: &BypassResult,
    stats: &Arc<ProxyStats>,
) -> io::Result<()> {
    let last = result.fragments.len().saturating_sub(1);

    for (i, fragment) in result.fragments.iter().enumerate() {
        remote.write_all(fragment).await?;
        stats
            .bytes_sent
            .fetch_add(fragment.len() as u64, Ordering::Relaxed);

        if i < last {
            if let Some(delay) = result.inter_fragment_delay {
                sleep(delay).await;
            }
        }
    }

    remote.flush().await
}

fn extract_connect_target(request: &str) -> io::Result<String> {
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Empty request"))?;

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

async fn relay_bidirectional(mut client: TcpStream, mut remote: TcpStream, stats: Arc<ProxyStats>) {
    match tokio::io::copy_bidirectional(&mut client, &mut remote).await {
        Ok((up, down)) => {
            stats.bytes_sent.fetch_add(up, Ordering::Relaxed);
            stats.bytes_received.fetch_add(down, Ordering::Relaxed);
        }
        Err(e) => {
            debug!("relay ended: {}", e);
        }
    }
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

    if let Some(without_scheme) = url.strip_prefix("http://") {
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
    session: &Session,
) -> io::Result<()> {
    let config = &session.config;
    let stats = &session.stats;
    if config.verbose {
        debug!("{} -> HTTP {}", peer_addr, target);
    }

    let mut remote = match connect_target(&target, session).await {
        Ok(stream) => stream,
        Err(e) => {
            let status = if e.kind() == ErrorKind::TimedOut {
                "504 Gateway Timeout"
            } else {
                "502 Bad Gateway"
            };
            let msg = format!("HTTP/1.1 {}\r\n\r\n{}\r\n", status, e);
            client.write_all(msg.as_bytes()).await?;
            return Err(e);
        }
    };

    let rewritten_request = rewrite_http_request(request, raw_request);

    let _ = remote.set_nodelay(true);

    let host = target.split(':').next().unwrap_or(&target);
    let engine = BypassEngine::new(config.bypass_for(host));
    let result = engine.process_outgoing(&rewritten_request);

    if let Some(ref host) = result.hostname {
        if result.modified {
            info!("🌐 {} [Host fragmented]", host);
        } else if config.verbose {
            debug!("🌐 {} [passthrough]", host);
        }
    }

    stats.http_connections.fetch_add(1, Ordering::Relaxed);
    if result.modified {
        stats.bypass_applied.fetch_add(1, Ordering::Relaxed);
    }

    write_fragments(&mut remote, &result, stats).await?;

    relay_bidirectional(client, remote, stats.clone()).await;

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

    let path = if let Some(without_scheme) = url.strip_prefix("http://") {
        if let Some(slash_pos) = without_scheme.find('/') {
            &without_scheme[slash_pos..]
        } else {
            "/"
        }
    } else {
        url
    };

    let header_end = match find_header_end(raw) {
        Some(end) => end,
        None => return raw.to_vec(),
    };

    let mut out = format!("{} {} {}\r\n", method, path, version).into_bytes();

    for line in request.lines().skip(1) {
        if line.is_empty() {
            break;
        }

        let name = line.split(':').next().unwrap_or("").trim().to_lowercase();
        if name == "proxy-connection" || name == "connection" || name == "keep-alive" {
            continue;
        }

        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out.extend_from_slice(&raw[header_end..]);
    out
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
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

    #[test]
    fn test_rewrite_http_request_forces_close() {
        let raw = b"GET http://discord.com/api HTTP/1.1\r\nHost: discord.com\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\n\r\n";
        let request = String::from_utf8_lossy(raw);
        let out = rewrite_http_request(&request, raw);
        let text = String::from_utf8(out).unwrap();

        assert!(text.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(text.contains("Host: discord.com\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(!text.to_lowercase().contains("proxy-connection"));
        assert!(!text.to_lowercase().contains("keep-alive"));
    }

    #[test]
    fn test_rewrite_http_request_keeps_body() {
        let raw = b"POST http://a.com/x HTTP/1.1\r\nHost: a.com\r\nContent-Length: 5\r\n\r\nhello";
        let request = String::from_utf8_lossy(raw);
        let out = rewrite_http_request(&request, raw);
        let text = String::from_utf8(out).unwrap();

        assert!(text.ends_with("\r\n\r\nhello"));
        assert!(text.contains("Content-Length: 5"));
    }
}
