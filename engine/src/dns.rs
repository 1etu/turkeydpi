use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::RwLock;
use std::time::{Duration, Instant};

pub struct DohResolver {
    cache: RwLock<HashMap<String, (Vec<IpAddr>, Instant)>>,
    ttl: Duration,
}

impl Default for DohResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DohResolver {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(300), 
        }
    }

    pub async fn resolve(&self, hostname: &str) -> std::io::Result<Vec<IpAddr>> {
        
        if let Some(ips) = self.get_cached(hostname) {
            return Ok(ips);
        }

        
        let providers = [
            ("1.1.1.1", "/dns-query"),           
            ("8.8.8.8", "/resolve"),              
            ("9.9.9.9", "/dns-query"),            
        ];

        for (server, path) in providers {
            match self.doh_query(server, path, hostname).await {
                Ok(ips) if !ips.is_empty() => {
                    self.cache_result(hostname, &ips);
                    return Ok(ips);
                }
                _ => continue,
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Failed to resolve {} via DoH", hostname),
        ))
    }

    pub async fn resolve_host_port(&self, host_port: &str) -> std::io::Result<SocketAddr> {
        let (host, port) = if let Some(idx) = host_port.rfind(':') {
            let port: u16 = host_port[idx + 1..].parse().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid port")
            })?;
            (&host_port[..idx], port)
        } else {
            (host_port, 443)
        };

        
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(SocketAddr::new(ip, port));
        }

        
        let ips = self.resolve(host).await?;
        
        
        let ip = ips.iter()
            .find(|ip| ip.is_ipv4())
            .or(ips.first())
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No IP addresses returned",
            ))?;

        Ok(SocketAddr::new(*ip, port))
    }

    fn get_cached(&self, hostname: &str) -> Option<Vec<IpAddr>> {
        let cache = self.cache.read().ok()?;
        let (ips, expiry) = cache.get(hostname)?;
        if Instant::now() < *expiry {
            Some(ips.clone())
        } else {
            None
        }
    }

    fn cache_result(&self, hostname: &str, ips: &[IpAddr]) {
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                hostname.to_string(),
                (ips.to_vec(), Instant::now() + self.ttl),
            );
        }
    }

    async fn doh_query(&self, server: &str, path: &str, hostname: &str) -> std::io::Result<Vec<IpAddr>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        
        let addr: SocketAddr = format!("{}:443", server).parse().unwrap();
        
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            TcpStream::connect(addr)
        ).await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DoH connect timeout"))?
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, e))?;

        
        let connector = tokio_native_tls::TlsConnector::from(
            native_tls::TlsConnector::new()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
        );

        let mut tls_stream = tokio::time::timeout(
            Duration::from_secs(5),
            connector.connect(server, stream)
        ).await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS timeout"))?
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        
        let request = format!(
            "GET {}?name={}&type=A HTTP/1.1\r\n\
             Host: {}\r\n\
             Accept: application/dns-json\r\n\
             Connection: close\r\n\r\n",
            path, hostname, server
        );

        tls_stream.write_all(request.as_bytes()).await?;
        tls_stream.flush().await?;

        
        let mut response = Vec::new();
        tls_stream.read_to_end(&mut response).await?;

        
        let response_str = String::from_utf8_lossy(&response);
        self.parse_doh_response(&response_str)
    }

    fn parse_doh_response(&self, response: &str) -> std::io::Result<Vec<IpAddr>> {
        
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
        
        let mut ips = Vec::new();
        
        
        
        for part in body.split("\"data\"") {
            if let Some(start) = part.find(":\"") {
                let rest = &part[start + 2..];
                if let Some(end) = rest.find('"') {
                    let ip_str = &rest[..end];
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        ips.push(ip);
                    }
                }
            }
        }

        Ok(ips)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cloudflare_response() {
        let resolver = DohResolver::new();
        let response = r#"HTTP/1.1 200 OK
Content-Type: application/dns-json

{"Status":0,"Answer":[{"name":"discord.com","type":1,"TTL":300,"data":"162.159.130.234"},{"name":"discord.com","type":1,"TTL":300,"data":"162.159.129.234"}]}"#;
        
        let ips = resolver.parse_doh_response(response).unwrap();
        assert!(!ips.is_empty());
        assert!(ips.iter().any(|ip| ip.to_string().starts_with("162.159")));
    }

    #[test]
    fn test_parse_google_response() {
        let resolver = DohResolver::new();
        let response = r#"HTTP/1.1 200 OK

{"Status":0,"Answer":[{"name":"discord.com.","type":1,"TTL":60,"data":"162.159.130.234"}]}"#;
        
        let ips = resolver.parse_doh_response(response).unwrap();
        assert!(!ips.is_empty());
    }
}
