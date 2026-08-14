use std::time::Duration;

use engine::DohResolver;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const LOOKUP_HOST: &str = "ipinfo.io";
const LOOKUP_PATH: &str = "/json";
const TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Deserialize)]
struct IpInfo {
    org: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub preset: Option<&'static str>,
}

pub fn match_preset(org: &str) -> Option<&'static str> {
    let lower = org.to_lowercase();

    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    if has(&["turk telekom", "türk telekom", "ttnet", "as9121", "as47331"]) {
        return Some("turk-telekom");
    }

    if has(&["vodafone", "as15897", "as8386"]) {
        return Some("vodafone");
    }

    if has(&["superonline", "turkcell", "as34984", "as16135"]) {
        return Some("superonline");
    }

    None
}

fn clean_org(org: &str) -> String {
    let trimmed = org.trim();

    let without_asn = trimmed
        .split_once(' ')
        .filter(|(head, _)| head.to_lowercase().starts_with("as"))
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);

    without_asn.trim().to_string()
}

pub async fn detect() -> Option<Provider> {
    let body = fetch().await?;
    let info: IpInfo = serde_json::from_str(&body).ok()?;

    let org = info.org?;
    let name = clean_org(&org);

    Some(Provider {
        preset: match_preset(&org),
        name,
    })
}

async fn fetch() -> Option<String> {
    let resolver = DohResolver::new();
    let addrs = resolver
        .resolve_socket_addrs(&format!("{}:443", LOOKUP_HOST))
        .await
        .ok()?;

    let mut stream = None;
    for addr in addrs {
        if let Ok(Ok(connected)) = tokio::time::timeout(TIMEOUT, TcpStream::connect(addr)).await {
            stream = Some(connected);
            break;
        }
    }

    let stream = stream?;

    let connector = tokio_native_tls::TlsConnector::from(native_tls::TlsConnector::new().ok()?);

    let mut tls = tokio::time::timeout(TIMEOUT, connector.connect(LOOKUP_HOST, stream))
        .await
        .ok()?
        .ok()?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nUser-Agent: turkeydpi\r\nConnection: close\r\n\r\n",
        LOOKUP_PATH, LOOKUP_HOST
    );

    tls.write_all(request.as_bytes()).await.ok()?;
    tls.flush().await.ok()?;

    let mut response = Vec::new();
    tokio::time::timeout(TIMEOUT, async {
        let mut chunk = [0u8; 4096];
        loop {
            let read = tls.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response.len() > 64 * 1024 {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    })
    .await
    .ok()?
    .ok()?;

    let text = String::from_utf8_lossy(&response);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");

    let start = body.find('{')?;
    let end = body.rfind('}')?;

    Some(body[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_known_providers() {
        assert_eq!(match_preset("AS9121 Turk Telekom"), Some("turk-telekom"));
        assert_eq!(match_preset("AS47331 TTNet A.S."), Some("turk-telekom"));
        assert_eq!(
            match_preset("AS15897 Vodafone Net Iletisim"),
            Some("vodafone")
        );
        assert_eq!(
            match_preset("AS34984 Superonline Iletisim"),
            Some("superonline")
        );
        assert_eq!(match_preset("AS16135 Turkcell"), Some("superonline"));
    }

    #[test]
    fn test_unknown_provider() {
        assert_eq!(match_preset("AS15169 Google LLC"), None);
        assert_eq!(match_preset(""), None);
    }

    #[test]
    fn test_clean_org_strips_asn() {
        assert_eq!(clean_org("AS9121 Turk Telekom"), "Turk Telekom");
        assert_eq!(clean_org("  Turk Telekom  "), "Turk Telekom");
        assert_eq!(clean_org("Cloudflare"), "Cloudflare");
    }
}
