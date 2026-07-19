use std::io::{Read, Write};
use std::net::{Ipv6Addr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const MAX_DEVICE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct DeviceClient {
    host_header: String,
    addr: String,
    timeout: Duration,
}

impl DeviceClient {
    pub(crate) fn new(host: &str, timeout: Duration) -> Result<Self, String> {
        let trimmed = host
            .strip_prefix("http://")
            .or_else(|| host.strip_prefix("https://"))
            .unwrap_or(host)
            .trim_end_matches('/');
        if trimmed.is_empty() {
            return Err("empty device host".to_string());
        }
        if host.starts_with("https://") {
            return Err(
                "https is not supported yet; MOTU device control is expected over http".to_string(),
            );
        }
        if trimmed.contains(['/', '?', '#']) || trimmed.chars().any(char::is_whitespace) {
            return Err("device host must not include a path, query, or whitespace".to_string());
        }
        let (host_header, addr) = parse_device_address(trimmed)?;
        Ok(Self {
            host_header,
            addr,
            timeout,
        })
    }

    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let path = normalize_path(path)?;
        let mut stream = connect_with_timeout(&self.addr, self.timeout)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| format!("set read timeout failed: {err}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| format!("set write timeout failed: {err}"))?;

        let content_headers = body.map_or_else(String::new, |body| {
            format!(
                "Content-Type: application/x-www-form-urlencoded; charset=utf-8\r\n\
                 Content-Length: {}\r\n",
                body.len()
            )
        });
        let request = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: cuemix-848/0.1\r\n\
             Accept: application/json, text/plain, */*\r\n\
             Connection: close\r\n\
             {}\r\n{}",
            self.host_header,
            content_headers,
            body.unwrap_or("")
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("write request failed: {err}"))?;

        let bytes = read_response(&mut stream, self.timeout)?;
        parse_http_response(&bytes)
    }
}

fn read_response(stream: &mut TcpStream, timeout: Duration) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if started.elapsed() > timeout {
            return Err("read response exceeded overall timeout".to_string());
        }
        let count = stream
            .read(&mut buffer)
            .map_err(|err| format!("read response failed: {err}"))?;
        if count == 0 {
            return Ok(bytes);
        }
        let new_length = bytes
            .len()
            .checked_add(count)
            .ok_or("response size overflow")?;
        if new_length > MAX_DEVICE_RESPONSE_BYTES {
            return Err(format!(
                "device response exceeds {} byte limit",
                MAX_DEVICE_RESPONSE_BYTES
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn parse_device_address(host: &str) -> Result<(String, String), String> {
    if let Some(rest) = host.strip_prefix('[') {
        let closing = rest
            .find(']')
            .ok_or("unterminated bracketed IPv6 address")?;
        let address = &rest[..closing];
        address
            .parse::<Ipv6Addr>()
            .map_err(|_| "invalid bracketed IPv6 address")?;
        let suffix = &rest[closing + 1..];
        let port = if suffix.is_empty() {
            "80"
        } else {
            suffix
                .strip_prefix(':')
                .filter(|port| !port.is_empty())
                .ok_or("invalid bracketed IPv6 host; expected [address]:port")?
        };
        port.parse::<u16>()
            .map_err(|_| "invalid bracketed IPv6 port")?;
        let host_header = if port == "80" {
            format!("[{address}]")
        } else {
            format!("[{address}]:{port}")
        };
        return Ok((host_header, format!("[{address}]:{port}")));
    }

    if host.parse::<Ipv6Addr>().is_ok() {
        return Ok((format!("[{host}]"), format!("[{host}]:80")));
    }

    if let Some((name, port)) = host.rsplit_once(':') {
        if name.is_empty() || port.parse::<u16>().is_err() {
            return Err("invalid device host; expected host or host:port".to_string());
        }
        return Ok((host.to_string(), host.to_string()));
    }

    Ok((host.to_string(), format!("{host}:80")))
}

fn connect_with_timeout(addr: &str, timeout: Duration) -> Result<TcpStream, String> {
    let addresses = addr
        .to_socket_addrs()
        .map_err(|err| format!("resolve {addr} failed: {err}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(format!("resolve {addr} returned no addresses"));
    }

    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_error = Some(err),
        }
    }
    let err = last_error.ok_or("no connection addresses were attempted")?;
    Err(format!("connect {addr} failed: {err}"))
}

#[derive(Debug, Clone)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) reason: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

pub(crate) fn parse_http_response(bytes: &[u8]) -> Result<HttpResponse, String> {
    let raw = String::from_utf8_lossy(bytes);
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or("response did not contain HTTP header separator")?;
    let mut lines = head.lines();
    let status_line = lines.next().ok_or("empty HTTP response")?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err("invalid HTTP version".to_string());
    }
    let status = status_parts
        .next()
        .ok_or("missing HTTP status")?
        .parse::<u16>()
        .map_err(|_| "invalid HTTP status")?;
    let reason = status_parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
            headers.push((name.trim().to_string(), value));
        }
    }
    let body = if chunked {
        decode_chunked(body.as_bytes()).ok_or("invalid chunked HTTP response")?
    } else {
        body.to_string()
    };
    Ok(HttpResponse {
        status,
        reason,
        headers,
        body,
    })
}

fn decode_chunked(bytes: &[u8]) -> Option<String> {
    let mut out = Vec::new();
    let mut index = 0;
    loop {
        let line_end = find_bytes(bytes, index, b"\r\n")?;
        let size_line = std::str::from_utf8(&bytes[index..line_end]).ok()?;
        let size_hex = size_line.split(';').next()?.trim();
        let size = usize::from_str_radix(size_hex, 16).ok()?;
        index = line_end + 2;
        if size == 0 {
            break;
        }
        let data_end = index.checked_add(size)?;
        if data_end > bytes.len() || bytes.get(data_end..data_end + 2) != Some(b"\r\n") {
            return None;
        }
        out.extend_from_slice(&bytes[index..data_end]);
        index = data_end + 2;
    }
    Some(String::from_utf8_lossy(&out).to_string())
}

fn find_bytes(haystack: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    haystack[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| start + position)
}

fn normalize_path(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("request path must be a whitespace-free origin path".to_string());
    }
    if path.starts_with("//") {
        return Err("request path must not be an authority path".to_string());
    }
    Ok(if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    })
}

pub(crate) fn datastore_set_body(value: &str) -> String {
    let object = format!("{{\"value\":{}}}", json_value(value));
    format!("json={object}")
}

pub(crate) fn datastore_write_request(path: &str, value: &str) -> Result<(String, String), String> {
    let path = normalize_path(path)?;
    if let Some(key) = path.strip_prefix("/datastore/") {
        // Firmware 2.3 accepts this raw root-body form, but ignores individual
        // datastore-path writes and percent-encoded inner JSON.
        let object = format!("{{\"{}\":{}}}", json_escape(key), json_value(value));
        return Ok(("/datastore".to_string(), format!("json={object}")));
    }
    Ok((path, datastore_set_body(value)))
}

fn json_value(value: &str) -> String {
    let trimmed = value.trim();
    if matches!(trimmed, "true" | "false" | "null") || is_json_number(trimmed) {
        trimmed.to_string()
    } else {
        format!("\"{}\"", json_escape(value))
    }
}

fn is_json_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                index += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                    if let Ok(value) = u8::from_str_radix(hex, 16) {
                        out.push(value);
                        index += 3;
                        continue;
                    }
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub(crate) fn json_escape(input: &str) -> String {
    let mut out = String::new();
    for character in input.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                out.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_datastore_set_body() {
        assert_eq!(datastore_set_body("-12"), "json={\"value\":-12}");
        assert_eq!(
            datastore_set_body("Main out"),
            "json={\"value\":\"Main out\"}"
        );
        assert_eq!(datastore_set_body("true"), "json={\"value\":true}");
    }

    #[test]
    fn quotes_non_json_number_values() {
        assert_eq!(datastore_set_body("01"), "json={\"value\":\"01\"}");
        assert_eq!(datastore_set_body("NaN"), "json={\"value\":\"NaN\"}");
        assert_eq!(
            datastore_set_body("{not valid}"),
            "json={\"value\":\"{not valid}\"}"
        );
        assert_eq!(datastore_set_body("-1.25e+3"), "json={\"value\":-1.25e+3}");
    }

    #[test]
    fn sends_datastore_key_updates_to_the_root() {
        assert_eq!(
            datastore_write_request("/datastore/ext/ibank/0/ch/0/trim", "51").unwrap(),
            (
                "/datastore".to_string(),
                "json={\"ext/ibank/0/ch/0/trim\":51}".to_string()
            )
        );
        assert_eq!(
            datastore_write_request("/custom/value", "on").unwrap(),
            ("/custom/value".to_string(), datastore_set_body("on"))
        );
    }

    #[test]
    fn parses_simple_http_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        let response = parse_http_response(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "hello");
    }

    #[test]
    fn decodes_chunked_body_and_rejects_missing_delimiters() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let response = parse_http_response(raw).unwrap();
        assert_eq!(response.body, "hello");
        assert_eq!(decode_chunked(b"1\r\naX0\r\n\r\n"), None);
    }

    #[test]
    fn parses_ipv6_hosts_and_rejects_paths() {
        assert_eq!(
            parse_device_address("[2604:4080:1503:8036::1]"),
            Ok((
                "[2604:4080:1503:8036::1]".to_string(),
                "[2604:4080:1503:8036::1]:80".to_string()
            ))
        );
        assert_eq!(
            parse_device_address("[2604:4080:1503:8036::1]:8080"),
            Ok((
                "[2604:4080:1503:8036::1]:8080".to_string(),
                "[2604:4080:1503:8036::1]:8080".to_string()
            ))
        );
        assert!(DeviceClient::new("http://848.local/control", Duration::from_secs(1)).is_err());
    }

    #[test]
    fn rejects_request_target_injection() {
        assert!(datastore_write_request("/datastore/ext\r\nX-Test: injected", "0").is_err());
        assert!(datastore_write_request("//other-device/path", "0").is_err());
        assert_eq!(normalize_path("apiversion"), Ok("/apiversion".to_string()));
    }
}
