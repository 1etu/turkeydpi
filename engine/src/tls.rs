use bytes::BytesMut;

pub const TLS_HANDSHAKE: u8 = 0x16;
pub const TLS_CHANGE_CIPHER_SPEC: u8 = 0x14;
pub const TLS_ALERT: u8 = 0x15;
pub const TLS_APPLICATION_DATA: u8 = 0x17;

pub const HANDSHAKE_CLIENT_HELLO: u8 = 0x01;
pub const HANDSHAKE_SERVER_HELLO: u8 = 0x02;

pub const EXT_SERVER_NAME: u16 = 0x0000;
pub const EXT_STATUS_REQUEST: u16 = 0x0005;
pub const EXT_SUPPORTED_GROUPS: u16 = 0x000a;
pub const EXT_EC_POINT_FORMATS: u16 = 0x000b;
pub const EXT_SIGNATURE_ALGORITHMS: u16 = 0x000d;

pub const SNI_HOST_NAME: u8 = 0x00;

#[derive(Debug, Clone)]
pub struct ClientHelloInfo {
    pub record_offset: usize,
    pub record_length: usize,
    pub sni_offset: Option<usize>,
    pub sni_length: Option<usize>,    
    pub sni_hostname: Option<String>,    
    pub record_version: (u8, u8),    
    pub client_version: (u8, u8),    
    pub is_valid: bool,
}

impl Default for ClientHelloInfo {
    fn default() -> Self {
        Self {
            record_offset: 0,
            record_length: 0,
            sni_offset: None,
            sni_length: None,
            sni_hostname: None,
            record_version: (0, 0),
            client_version: (0, 0),
            is_valid: false,
        }
    }
}

impl ClientHelloInfo {
    pub fn get_split_points(&self) -> Vec<usize> {
        let mut points = Vec::new();
        
        if let Some(sni_offset) = self.sni_offset {
            if sni_offset > 5 {
                points.push(sni_offset - 1);
            }
            
            if let Some(sni_len) = self.sni_length {
                if sni_len > 4 {
                    points.push(sni_offset + sni_len / 2);
                }
            }
        }
        
        points
    }
    
    pub fn get_turkey_split_point(&self) -> Option<usize> {
        if self.is_valid && self.record_length > 10 {
            Some(self.record_offset + 3)
        } else {
            None
        }
    }
}

pub fn parse_client_hello(data: &[u8]) -> Option<ClientHelloInfo> {
    let mut info = ClientHelloInfo::default();
    
    if data.len() < 6 {
        return None;
    }
    
    let mut pos = 0;
    let content_type = data[pos];

    if content_type != TLS_HANDSHAKE {
        return None;
    }
    pos += 1;
    
    info.record_version = (data[pos], data[pos + 1]);
    pos += 2;
    
    let record_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    info.record_length = record_length + 5;
    
    if data.len() < pos + record_length {
      
    }
    
    if pos >= data.len() {
        return None;
    }
    
    let handshake_type = data[pos];
    if handshake_type != HANDSHAKE_CLIENT_HELLO {
        return None;
    }
    pos += 1;
    
    info.is_valid = true;
    
    if pos + 3 > data.len() {
        return Some(info);
    }
    let _handshake_length = u32::from_be_bytes([0, data[pos], data[pos + 1], data[pos + 2]]) as usize;
    pos += 3;
    
    if pos + 2 > data.len() {
        return Some(info);
    }
    info.client_version = (data[pos], data[pos + 1]);
    pos += 2;
    
    pos += 32;
    if pos > data.len() {
        return Some(info);
    }
    
    if pos >= data.len() {
        return Some(info);
    }
    let session_id_len = data[pos] as usize;
    pos += 1 + session_id_len;
    if pos > data.len() {
        return Some(info);
    }
    
    if pos + 2 > data.len() {
        return Some(info);
    }
    let cipher_suites_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2 + cipher_suites_len;
    if pos > data.len() {
        return Some(info);
    }
    
    if pos >= data.len() {
        return Some(info);
    }
    let compression_len = data[pos] as usize;
    pos += 1 + compression_len;
    if pos > data.len() {
        return Some(info);
    }
    
    if pos + 2 > data.len() {
        return Some(info);
    }
    let extensions_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    
    let extensions_end = pos + extensions_len;
    
    while pos + 4 <= data.len() && pos < extensions_end {
        let ext_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;
        
        let ext_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        
        if ext_type == EXT_SERVER_NAME {
            if pos + 5 <= data.len() && pos + ext_len <= data.len() {
                let _sni_list_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                let name_type = data[pos + 2];
                let name_len = u16::from_be_bytes([data[pos + 3], data[pos + 4]]) as usize;
                
                if name_type == SNI_HOST_NAME {
                    let name_offset = pos + 5;
                    info.sni_offset = Some(name_offset);
                    info.sni_length = Some(name_len);
                    
                    if name_offset + name_len <= data.len() {
                        if let Ok(hostname) = std::str::from_utf8(&data[name_offset..name_offset + name_len]) {
                            info.sni_hostname = Some(hostname.to_string());
                        }
                    }
                }
            }
            break;
        }
        
        pos += ext_len;
    }
    
    Some(info)
}

pub fn is_client_hello(data: &[u8]) -> bool {
    if data.len() < 6 {
        return false;
    }
    
    if data[0] != TLS_HANDSHAKE {
        return false;
    }
    
    if data[1] != 0x03 || data[2] > 0x04 {
        return false;
    }
    
    if data[5] != HANDSHAKE_CLIENT_HELLO {
        return false;
    }
    
    true
}

pub fn is_http_request(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    
    data.starts_with(b"GET ") ||
    data.starts_with(b"POST") ||
    data.starts_with(b"HEAD") ||
    data.starts_with(b"PUT ") ||
    data.starts_with(b"DELETE") ||
    data.starts_with(b"OPTIONS") ||
    data.starts_with(b"CONNECT") ||
    data.starts_with(b"PATCH")
}

pub fn find_http_host(data: &[u8]) -> Option<(usize, usize)> {
    let text = std::str::from_utf8(data).ok()?;
    
    let lower = text.to_lowercase();
    let host_pos = lower.find("\nhost:")?;
    
    let value_start = host_pos + 6;
    
    let mut start = value_start;
    while start < text.len() && (text.as_bytes()[start] == b' ' || text.as_bytes()[start] == b'\t') {
        start += 1;
    }
    
    let end = text[start..].find('\r')
        .or_else(|| text[start..].find('\n'))
        .map(|p| start + p)
        .unwrap_or(text.len());
    
    Some((start, end - start))
}

pub fn fragment_at_offsets(data: &[u8], offsets: &[usize]) -> Vec<BytesMut> {
    let mut fragments = Vec::new();
    let mut prev = 0;
    
    let mut sorted_offsets: Vec<usize> = offsets.iter()
        .filter(|&&o| o > 0 && o < data.len())
        .copied()
        .collect();
    sorted_offsets.sort();
    sorted_offsets.dedup();
    
    for offset in sorted_offsets {
        if offset > prev && offset <= data.len() {
            fragments.push(BytesMut::from(&data[prev..offset]));
            prev = offset;
        }
    }
    
    if prev < data.len() {
        fragments.push(BytesMut::from(&data[prev..]));
    }
    
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn sample_client_hello() -> Vec<u8> {
        vec![
            0x16, 
            0x03, 0x01, 
            0x00, 0xf1, 
            
            0x01, 
            0x00, 0x00, 0xed, 
            
            
            0x03, 0x03, 
            
            
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
            0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
            
            
            0x00, 
            
            
            0x00, 0x04, 
            0x13, 0x01, 
            0x13, 0x02, 
            
            
            0x01, 
            0x00, 
            
            
            0x00, 0x1e, 
            
            
            0x00, 0x00, 
            0x00, 0x10, 
            0x00, 0x0e, 
            0x00, 
            0x00, 0x0b, 
            0x64, 0x69, 0x73, 0x63, 0x6f, 0x72, 0x64, 0x2e, 0x63, 0x6f, 0x6d,
            
            
            0x00, 0x15, 
            0x00, 0x06, 
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]
    }
    
    #[test]
    fn test_is_client_hello() {
        let data = sample_client_hello();
        assert!(is_client_hello(&data));
        
        assert!(!is_client_hello(&[0x17, 0x03, 0x01, 0x00, 0x10, 0x00]));
        assert!(!is_client_hello(b"GET / HTTP/1.1"));
    }
    
    #[test]
    fn test_parse_client_hello() {
        let data = sample_client_hello();
        let info = parse_client_hello(&data).unwrap();
        
        assert!(info.is_valid);
        assert_eq!(info.record_version, (0x03, 0x01));
        assert_eq!(info.client_version, (0x03, 0x03));
        assert!(info.sni_offset.is_some());
        assert_eq!(info.sni_hostname.as_deref(), Some("discord.com"));
    }
    
    #[test]
    fn test_get_split_points() {
        let data = sample_client_hello();
        let info = parse_client_hello(&data).unwrap();
        
        let points = info.get_split_points();
        assert!(!points.is_empty());
        
        for point in &points {
            assert!(*point < data.len());
        }
    }
    
    #[test]
    fn test_turkey_split_point() {
        let data = sample_client_hello();
        let info = parse_client_hello(&data).unwrap();
        
        let split = info.get_turkey_split_point();
        assert!(split.is_some());
        assert!(split.unwrap() < 10);
    }
    
    #[test]
    fn test_is_http_request() {
        assert!(is_http_request(b"GET / HTTP/1.1\r\n"));
        assert!(is_http_request(b"POST /api HTTP/1.1\r\n"));
        assert!(!is_http_request(b"\x16\x03\x01"));
        assert!(!is_http_request(b"HTTP/1.1 200")); 
    }
    
    #[test]
    fn test_find_http_host() {
        let request = b"GET / HTTP/1.1\r\nHost: discord.com\r\nConnection: close\r\n\r\n";
        let (offset, len) = find_http_host(request).unwrap();
        
        let host = std::str::from_utf8(&request[offset..offset + len]).unwrap();
        assert_eq!(host, "discord.com");
    }
    
    #[test]
    fn test_fragment_at_offsets() {
        let data = b"Hello, World!";
        
        let fragments = fragment_at_offsets(data, &[5, 7]);
        assert_eq!(fragments.len(), 3);
        assert_eq!(&fragments[0][..], b"Hello");
        assert_eq!(&fragments[1][..], b", ");
        assert_eq!(&fragments[2][..], b"World!");
    }
}
