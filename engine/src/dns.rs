use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use serde::Deserialize;

const DEFAULT_CACHE_SIZE: usize = 1024;
const MIN_TTL: Duration = Duration::from_secs(30);
const MAX_TTL: Duration = Duration::from_secs(3600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;

struct Provider {
    addr: &'static str,
    path: &'static str,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        addr: "1.1.1.1",
        path: "/dns-query",
    },
    Provider {
        addr: "9.9.9.9",
        path: "/dns-query",
    },
    Provider {
        addr: "8.8.8.8",
        path: "/resolve",
    },
];

#[derive(Debug, Clone)]
struct CacheEntry {
    ips: Vec<IpAddr>,
    expires: Instant,
}

#[derive(Debug, Deserialize)]
struct DohAnswer {
    #[serde(rename = "type")]
    rtype: u16,
    #[serde(rename = "TTL")]
    ttl: Option<u64>,
    data: String,
}

#[derive(Debug, Deserialize)]
struct DohResponse {
    #[serde(rename = "Status")]
    status: i32,
    #[serde(rename = "Answer")]
    answer: Option<Vec<DohAnswer>>,
}

pub struct DohResolver {
    cache: Mutex<LruCache<String, CacheEntry>>,
}

impl Default for DohResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DohResolver {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CACHE_SIZE)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let size = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            cache: Mutex::new(LruCache::new(size)),
        }
    }

    pub fn cache_len(&self) -> usize {
        self.cache.lock().len()
    }

    pub async fn resolve(&self, hostname: &str) -> std::io::Result<Vec<IpAddr>> {
        if let Some(ips) = self.get_cached(hostname) {
            return Ok(ips);
        }

        let mut last_error = None;

        for provider in PROVIDERS {
            match self.query_provider(provider, hostname).await {
                Ok((ips, ttl)) if !ips.is_empty() => {
                    self.store(hostname, &ips, ttl);
                    return Ok(ips);
                }
                Ok(_) => continue,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no DoH provider resolved {}", hostname),
            )
        }))
    }

    pub async fn resolve_socket_addrs(&self, host_port: &str) -> std::io::Result<Vec<SocketAddr>> {
        let (host, port) = split_host_port(host_port)?;

        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }

        let ips = self.resolve(&host).await?;

        let mut addrs: Vec<SocketAddr> = Vec::with_capacity(ips.len());
        addrs.extend(
            ips.iter()
                .filter(|ip| ip.is_ipv4())
                .map(|ip| SocketAddr::new(*ip, port)),
        );
        addrs.extend(
            ips.iter()
                .filter(|ip| ip.is_ipv6())
                .map(|ip| SocketAddr::new(*ip, port)),
        );

        if addrs.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no addresses returned",
            ));
        }

        Ok(addrs)
    }

    pub async fn resolve_host_port(&self, host_port: &str) -> std::io::Result<SocketAddr> {
        let addrs = self.resolve_socket_addrs(host_port).await?;
        Ok(addrs[0])
    }

    fn get_cached(&self, hostname: &str) -> Option<Vec<IpAddr>> {
        let mut cache = self.cache.lock();
        let entry = cache.get(hostname)?;

        if Instant::now() < entry.expires {
            return Some(entry.ips.clone());
        }

        cache.pop(hostname);
        None
    }

    fn store(&self, hostname: &str, ips: &[IpAddr], ttl: Duration) {
        let entry = CacheEntry {
            ips: ips.to_vec(),
            expires: Instant::now() + ttl,
        };
        self.cache.lock().put(hostname.to_string(), entry);
    }

    async fn query_provider(
        &self,
        provider: &Provider,
        hostname: &str,
    ) -> std::io::Result<(Vec<IpAddr>, Duration)> {
        let (v4, v6) = tokio::join!(
            self.doh_query(provider, hostname, TYPE_A),
            self.doh_query(provider, hostname, TYPE_AAAA)
        );

        let mut ips = Vec::new();
        let mut ttl = MAX_TTL;

        if let Ok((mut found, found_ttl)) = v4 {
            ips.append(&mut found);
            ttl = ttl.min(found_ttl);
        }

        if let Ok((mut found, found_ttl)) = v6 {
            ips.append(&mut found);
            ttl = ttl.min(found_ttl);
        }

        if ips.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "empty answer",
            ));
        }

        Ok((ips, ttl.clamp(MIN_TTL, MAX_TTL)))
    }

    async fn doh_query(
        &self,
        provider: &Provider,
        hostname: &str,
        rtype: u16,
    ) -> std::io::Result<(Vec<IpAddr>, Duration)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let addr: SocketAddr = format!("{}:443", provider.addr)
            .parse()
            .map_err(|_| std::io::Error::other("bad provider address"))?;

        let stream = tokio::time::timeout(QUERY_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "DoH connect timeout")
            })??;

        let connector = tokio_native_tls::TlsConnector::from(
            native_tls::TlsConnector::new().map_err(std::io::Error::other)?,
        );

        let mut tls = tokio::time::timeout(QUERY_TIMEOUT, connector.connect(provider.addr, stream))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DoH TLS timeout"))?
            .map_err(std::io::Error::other)?;

        let request = format!(
            "GET {}?name={}&type={} HTTP/1.1\r\n\
             Host: {}\r\n\
             Accept: application/dns-json\r\n\
             User-Agent: turkeydpi\r\n\
             Connection: close\r\n\r\n",
            provider.path, hostname, rtype, provider.addr
        );

        tls.write_all(request.as_bytes()).await?;
        tls.flush().await?;

        let mut response = Vec::new();
        tokio::time::timeout(QUERY_TIMEOUT, async {
            let mut chunk = [0u8; 4096];
            loop {
                let n = tls.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                response.extend_from_slice(&chunk[..n]);
                if response.len() > MAX_RESPONSE_BYTES {
                    break;
                }
            }
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DoH read timeout"))??;

        let text = String::from_utf8_lossy(&response);
        parse_doh_response(&text, rtype)
    }
}

fn split_host_port(host_port: &str) -> std::io::Result<(String, u16)> {
    if let Some(rest) = host_port.strip_prefix('[') {
        let end = rest.find(']').ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unterminated ipv6 literal",
            )
        })?;

        let host = rest[..end].to_string();
        let port = match rest[end + 1..].strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid port")
            })?,
            None => 443,
        };

        return Ok((host, port));
    }

    match host_port.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            let port = port.parse::<u16>().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid port")
            })?;
            Ok((host.to_string(), port))
        }
        _ => Ok((host_port.to_string(), 443)),
    }
}

fn split_http_body(response: &str) -> &str {
    if let Some(pos) = response.find("\r\n\r\n") {
        &response[pos + 4..]
    } else if let Some(pos) = response.find("\n\n") {
        &response[pos + 2..]
    } else {
        ""
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;

    loop {
        let line_end = match rest.find("\r\n") {
            Some(pos) => pos,
            None => break,
        };

        let size = match usize::from_str_radix(rest[..line_end].trim(), 16) {
            Ok(size) => size,
            Err(_) => return body.to_string(),
        };

        if size == 0 {
            break;
        }

        let start = line_end + 2;
        let end = start + size;
        if end > rest.len() {
            return body.to_string();
        }

        out.push_str(&rest[start..end]);
        rest = rest[end..].trim_start_matches("\r\n");
    }

    out
}

fn parse_doh_response(response: &str, rtype: u16) -> std::io::Result<(Vec<IpAddr>, Duration)> {
    let body = split_http_body(response).trim();

    let json = if body.starts_with('{') {
        body.to_string()
    } else {
        dechunk(body)
    };

    let parsed: DohResponse = serde_json::from_str(json.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if parsed.status != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("DoH status {}", parsed.status),
        ));
    }

    let mut ips = Vec::new();
    let mut ttl = MAX_TTL;

    for answer in parsed.answer.unwrap_or_default() {
        if answer.rtype != rtype {
            continue;
        }

        if let Ok(ip) = answer.data.parse::<IpAddr>() {
            ips.push(ip);
            if let Some(secs) = answer.ttl {
                ttl = ttl.min(Duration::from_secs(secs));
            }
        }
    }

    Ok((ips, ttl.clamp(MIN_TTL, MAX_TTL)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cloudflare_response() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/dns-json\r\n\r\n{\"Status\":0,\"Answer\":[{\"name\":\"discord.com\",\"type\":1,\"TTL\":300,\"data\":\"162.159.130.234\"},{\"name\":\"discord.com\",\"type\":1,\"TTL\":120,\"data\":\"162.159.129.234\"}]}";

        let (ips, ttl) = parse_doh_response(response, TYPE_A).unwrap();

        assert_eq!(ips.len(), 2);
        assert_eq!(ttl, Duration::from_secs(120));
    }

    #[test]
    fn test_parse_lf_only_headers() {
        let response = "HTTP/1.1 200 OK\n\n{\"Status\":0,\"Answer\":[{\"name\":\"a.\",\"type\":1,\"TTL\":60,\"data\":\"1.2.3.4\"}]}";

        let (ips, ttl) = parse_doh_response(response, TYPE_A).unwrap();

        assert_eq!(ips, vec!["1.2.3.4".parse::<IpAddr>().unwrap()]);
        assert_eq!(ttl, Duration::from_secs(60));
    }

    #[test]
    fn test_parse_ignores_other_record_types() {
        let response = "HTTP/1.1 200 OK\r\n\r\n{\"Status\":0,\"Answer\":[{\"name\":\"a.\",\"type\":5,\"TTL\":60,\"data\":\"cname.example.com\"},{\"name\":\"a.\",\"type\":1,\"TTL\":60,\"data\":\"1.2.3.4\"}]}";

        let (ips, _) = parse_doh_response(response, TYPE_A).unwrap();

        assert_eq!(ips.len(), 1);
    }

    #[test]
    fn test_parse_aaaa() {
        let response = "HTTP/1.1 200 OK\r\n\r\n{\"Status\":0,\"Answer\":[{\"name\":\"a.\",\"type\":28,\"TTL\":90,\"data\":\"2606:4700::1\"}]}";

        let (ips, _) = parse_doh_response(response, TYPE_AAAA).unwrap();

        assert_eq!(ips.len(), 1);
        assert!(ips[0].is_ipv6());
    }

    #[test]
    fn test_parse_nxdomain_is_error() {
        let response = "HTTP/1.1 200 OK\r\n\r\n{\"Status\":3}";

        assert!(parse_doh_response(response, TYPE_A).is_err());
    }

    #[test]
    fn test_parse_chunked_body() {
        let json = "{\"Status\":0,\"Answer\":[{\"type\":1,\"TTL\":60,\"data\":\"1.2.3.4\"}]}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            json.len(),
            json
        );

        let (ips, _) = parse_doh_response(&response, TYPE_A).unwrap();

        assert_eq!(ips.len(), 1);
    }

    #[test]
    fn test_split_host_port() {
        assert_eq!(
            split_host_port("discord.com:443").unwrap(),
            ("discord.com".to_string(), 443)
        );
        assert_eq!(
            split_host_port("discord.com").unwrap(),
            ("discord.com".to_string(), 443)
        );
        assert_eq!(
            split_host_port("[2606:4700::1]:8443").unwrap(),
            ("2606:4700::1".to_string(), 8443)
        );
        assert_eq!(
            split_host_port("[2606:4700::1]").unwrap(),
            ("2606:4700::1".to_string(), 443)
        );
        assert!(split_host_port("discord.com:notaport").is_err());
    }

    #[test]
    fn test_cache_is_bounded() {
        let resolver = DohResolver::with_capacity(4);

        for i in 0..32 {
            resolver.store(
                &format!("host{}.example", i),
                &["1.2.3.4".parse().unwrap()],
                Duration::from_secs(60),
            );
        }

        assert_eq!(resolver.cache_len(), 4);
    }

    #[test]
    fn test_expired_entry_is_dropped() {
        let resolver = DohResolver::new();
        resolver.store(
            "stale.example",
            &["1.2.3.4".parse().unwrap()],
            Duration::from_millis(1),
        );

        std::thread::sleep(Duration::from_millis(10));

        assert!(resolver.get_cached("stale.example").is_none());
        assert_eq!(resolver.cache_len(), 0);
    }
}
