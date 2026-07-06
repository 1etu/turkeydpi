use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::bypass::{BypassConfig, BypassEngine};
use crate::dns::DohResolver;
use crate::tls::TLS_HANDSHAKE;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const READ_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Reachable,
    Blocked,
    DnsFailed,
    ConnectFailed,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub host: String,
    pub outcome: ProbeOutcome,
    pub elapsed: Duration,
    pub detail: Option<String>,
}

impl ProbeResult {
    pub fn is_reachable(&self) -> bool {
        self.outcome == ProbeOutcome::Reachable
    }
}

fn random_bytes(count: usize) -> Vec<u8> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5eed);

    let mut state = nanos | 1;
    let mut out = Vec::with_capacity(count);

    for _ in 0..count {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push((state >> 24) as u8);
    }

    out
}

fn push_u16_block(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
}

fn server_name_extension(host: &str) -> Vec<u8> {
    let mut name = Vec::new();
    name.push(0x00);
    push_u16_block(&mut name, host.as_bytes());

    let mut list = Vec::new();
    push_u16_block(&mut list, &name);

    let mut extension = Vec::new();
    extension.extend_from_slice(&0x0000u16.to_be_bytes());
    push_u16_block(&mut extension, &list);
    extension
}

fn extension(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&kind.to_be_bytes());
    push_u16_block(&mut out, body);
    out
}

pub fn build_client_hello(host: &str) -> Vec<u8> {
    let mut body = Vec::new();

    body.extend_from_slice(&[0x03, 0x03]);
    body.extend_from_slice(&random_bytes(32));
    body.push(0x00);

    let suites: [u16; 6] = [0xc02f, 0xc030, 0xc02b, 0xc02c, 0x009c, 0x009d];
    let mut suite_bytes = Vec::new();
    for suite in suites {
        suite_bytes.extend_from_slice(&suite.to_be_bytes());
    }
    push_u16_block(&mut body, &suite_bytes);

    body.push(0x01);
    body.push(0x00);

    let mut extensions = Vec::new();
    extensions.extend_from_slice(&server_name_extension(host));
    extensions.extend_from_slice(&extension(0x000a, &[0x00, 0x04, 0x00, 0x17, 0x00, 0x1d]));
    extensions.extend_from_slice(&extension(0x000b, &[0x01, 0x00]));
    extensions.extend_from_slice(&extension(
        0x000d,
        &[0x00, 0x08, 0x04, 0x01, 0x08, 0x04, 0x02, 0x01, 0x05, 0x01],
    ));
    extensions.extend_from_slice(&extension(0x0017, &[]));
    extensions.extend_from_slice(&extension(0xff01, &[0x00]));

    push_u16_block(&mut body, &extensions);

    let mut handshake = Vec::new();
    handshake.push(0x01);
    let length = body.len();
    handshake.push((length >> 16) as u8);
    handshake.push((length >> 8) as u8);
    handshake.push(length as u8);
    handshake.extend_from_slice(&body);

    let mut record = Vec::new();
    record.push(TLS_HANDSHAKE);
    record.extend_from_slice(&[0x03, 0x01]);
    push_u16_block(&mut record, &handshake);

    record
}

pub async fn probe_host(resolver: &DohResolver, host: &str, config: &BypassConfig) -> ProbeResult {
    let started = Instant::now();
    let target = format!("{}:443", host);

    let addrs = match resolver.resolve_socket_addrs(&target).await {
        Ok(addrs) => addrs,
        Err(e) => {
            return ProbeResult {
                host: host.to_string(),
                outcome: ProbeOutcome::DnsFailed,
                elapsed: started.elapsed(),
                detail: Some(e.to_string()),
            }
        }
    };

    let mut stream = None;
    let mut connect_error = None;

    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => {
                stream = Some(s);
                break;
            }
            Ok(Err(e)) => connect_error = Some(e.to_string()),
            Err(_) => connect_error = Some("connect timeout".to_string()),
        }
    }

    let mut stream = match stream {
        Some(stream) => stream,
        None => {
            return ProbeResult {
                host: host.to_string(),
                outcome: ProbeOutcome::ConnectFailed,
                elapsed: started.elapsed(),
                detail: connect_error,
            }
        }
    };

    let _ = stream.set_nodelay(true);

    let hello = build_client_hello(host);
    let engine = BypassEngine::new(config.clone());
    let result = engine.process_outgoing(&hello);

    let last = result.fragments.len().saturating_sub(1);
    for (i, fragment) in result.fragments.iter().enumerate() {
        if let Err(e) = stream.write_all(fragment).await {
            return ProbeResult {
                host: host.to_string(),
                outcome: ProbeOutcome::Blocked,
                elapsed: started.elapsed(),
                detail: Some(e.to_string()),
            };
        }

        if i < last {
            if let Some(delay) = result.inter_fragment_delay {
                tokio::time::sleep(delay).await;
            }
        }
    }

    if let Err(e) = stream.flush().await {
        return ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Blocked,
            elapsed: started.elapsed(),
            detail: Some(e.to_string()),
        };
    }

    let mut buf = [0u8; 8];
    match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf)).await {
        Ok(Ok(0)) => ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Blocked,
            elapsed: started.elapsed(),
            detail: Some("closed without reply".to_string()),
        },
        Ok(Ok(_)) if buf[0] == TLS_HANDSHAKE => ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Reachable,
            elapsed: started.elapsed(),
            detail: None,
        },
        Ok(Ok(_)) => ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Blocked,
            elapsed: started.elapsed(),
            detail: Some(format!("unexpected record type 0x{:02x}", buf[0])),
        },
        Ok(Err(e)) => ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Blocked,
            elapsed: started.elapsed(),
            detail: Some(e.to_string()),
        },
        Err(_) => ProbeResult {
            host: host.to_string(),
            outcome: ProbeOutcome::Blocked,
            elapsed: started.elapsed(),
            detail: Some("no reply".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::{is_client_hello, parse_client_hello};

    #[test]
    fn test_built_hello_is_valid() {
        let hello = build_client_hello("discord.com");

        assert!(is_client_hello(&hello));

        let info = parse_client_hello(&hello).unwrap();
        assert!(info.is_valid);
        assert_eq!(info.sni_hostname.as_deref(), Some("discord.com"));
    }

    #[test]
    fn test_record_length_matches_body() {
        let hello = build_client_hello("example.org");
        let declared = u16::from_be_bytes([hello[3], hello[4]]) as usize;

        assert_eq!(declared, hello.len() - 5);
    }

    #[test]
    fn test_handshake_length_matches_body() {
        let hello = build_client_hello("example.org");
        let declared = u32::from_be_bytes([0, hello[6], hello[7], hello[8]]) as usize;

        assert_eq!(declared, hello.len() - 9);
    }

    #[test]
    fn test_every_preset_fragments_the_probe() {
        let hello = build_client_hello("discord.com");

        for name in BypassConfig::preset_names() {
            let engine = BypassEngine::new(BypassConfig::preset(name).unwrap());
            let result = engine.process_outgoing(&hello);

            assert!(result.modified, "{} did not fragment the probe", name);

            let rebuilt: Vec<u8> = result
                .fragments
                .iter()
                .flat_map(|f| f.iter().copied())
                .collect();
            assert_eq!(rebuilt, hello);
        }
    }
}
