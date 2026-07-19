use std::io::{Read, Write};
use std::net::{Ipv6Addr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::device::{json_escape, parse_http_response, HttpResponse};

const APP_HEADER_LEN: usize = 10;
const CONNECT_HEADER_LIMIT: usize = 32 * 1024;
const INITIAL_FRAME_WAIT: Duration = Duration::from_millis(250);
const INITIAL_DATA_LIMIT: usize = 8 * 1024;

const APP_NOP: u8 = 0x00;
const APP_ENTITY_GUID_REQUEST: u8 = 0x01;
const APP_ENTITY_GUID_RESPONSE: u8 = 0x02;
const APP_LINK_UP: u8 = 0x03;
const APP_LINK_DOWN: u8 = 0x04;
const APP_AVDECC_FROM_APS: u8 = 0x05;
const APP_AVDECC_FROM_APC: u8 = 0x06;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppFrame {
    version: u8,
    message_type: u8,
    address: [u8; 6],
    payload: Vec<u8>,
}

pub(crate) struct AvdeccProbeResult {
    status: u16,
    reason: String,
    frames: Vec<AppFrame>,
    initial_data: Vec<u8>,
}

pub(crate) fn probe(
    host: &str,
    path: &str,
    timeout: Duration,
) -> Result<AvdeccProbeResult, String> {
    let mut proxy = AvdeccProxy::connect(host, path, timeout)?;
    let initial_data = proxy.read_available_for(timeout.min(INITIAL_FRAME_WAIT))?;
    Ok(AvdeccProbeResult {
        status: proxy.response.status,
        reason: proxy.response.reason,
        frames: decode_complete_v0_frames(&initial_data),
        initial_data,
    })
}

pub(crate) fn write_probe_result(result: &AvdeccProbeResult) {
    let frames = result
        .frames
        .iter()
        .map(app_frame_json)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"status\":{},\"reason\":\"{}\",\"initial_bytes\":{},\"initial_preview\":\"{}\",\"v0_frames\":[{}]}}",
        result.status,
        json_escape(&result.reason),
        result.initial_data.len(),
        hex_preview(&result.initial_data, 64),
        frames
    );
}

struct AvdeccProxy {
    stream: TcpStream,
    buffered: Vec<u8>,
    response: HttpResponse,
}

impl AvdeccProxy {
    fn connect(host: &str, path: &str, timeout: Duration) -> Result<Self, String> {
        let address = parse_proxy_address(host)?;
        validate_proxy_path(path)?;
        let mut stream = connect_with_timeout(&address.socket_address, timeout)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| format!("set AVDECC Proxy write timeout failed: {error}"))?;
        let request = format!(
            "CONNECT {path} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: cuemix-848/0.1\r\n\
             Connection: keep-alive\r\n\
             \r\n",
            address.host_header
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("write AVDECC Proxy CONNECT failed: {error}"))?;
        let (response, buffered) = read_connect_response(&mut stream, timeout)?;
        if response.status != 200 {
            return Err(format!(
                "AVDECC Proxy CONNECT returned HTTP {} {}",
                response.status, response.reason
            ));
        }
        Ok(Self {
            stream,
            buffered,
            response,
        })
    }

    fn read_available_for(&mut self, wait: Duration) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let mut data = std::mem::take(&mut self.buffered);
        while data.len() < INITIAL_DATA_LIMIT {
            let Some(remaining) = wait.checked_sub(started.elapsed()) else {
                break;
            };
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
            let mut buffer = [0u8; 1536];
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err("AVDECC Proxy closed the tunnel".to_string()),
                Ok(count) => {
                    let remaining_capacity = INITIAL_DATA_LIMIT - data.len();
                    data.extend_from_slice(&buffer[..count.min(remaining_capacity)]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    break
                }
                Err(error) => return Err(format!("read AVDECC Proxy tunnel failed: {error}")),
            }
        }
        Ok(data)
    }
}

fn hex_preview(bytes: &[u8], maximum: usize) -> String {
    bytes
        .iter()
        .take(maximum)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_connect_response(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<(HttpResponse, Vec<u8>), String> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    loop {
        if let Some(header_end) = find_header_end(&bytes) {
            let response = parse_http_response(&bytes[..header_end])?;
            return Ok((response, bytes[header_end..].to_vec()));
        }
        if bytes.len() >= CONNECT_HEADER_LIMIT {
            return Err("AVDECC Proxy CONNECT response headers are too large".to_string());
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or("timed out waiting for AVDECC Proxy CONNECT response")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
        let mut buffer = [0u8; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => return Err("AVDECC Proxy closed before CONNECT response".to_string()),
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                return Err("timed out waiting for AVDECC Proxy CONNECT response".to_string())
            }
            Err(error) => {
                return Err(format!(
                    "read AVDECC Proxy CONNECT response failed: {error}"
                ))
            }
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn decode_app_frame(header: &[u8], payload: Vec<u8>) -> Result<AppFrame, String> {
    if header.len() != APP_HEADER_LEN {
        return Err("invalid AVDECC Proxy frame header length".to_string());
    }
    let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
    if payload.len() != payload_len {
        return Err("invalid AVDECC Proxy frame payload length".to_string());
    }
    let address: [u8; 6] = header[4..10]
        .try_into()
        .map_err(|_| "invalid AVDECC Proxy frame address")?;
    Ok(AppFrame {
        version: header[0],
        message_type: header[1],
        address,
        payload,
    })
}

fn decode_complete_v0_frames(bytes: &[u8]) -> Vec<AppFrame> {
    let mut offset = 0;
    let mut frames = Vec::new();
    while bytes.get(offset) == Some(&0) && bytes.len() - offset >= APP_HEADER_LEN {
        let header = &bytes[offset..offset + APP_HEADER_LEN];
        let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        let frame_end = offset + APP_HEADER_LEN + payload_len;
        if frame_end > bytes.len() {
            break;
        }
        let payload = bytes[offset + APP_HEADER_LEN..frame_end].to_vec();
        let Ok(frame) = decode_app_frame(header, payload) else {
            break;
        };
        frames.push(frame);
        offset = frame_end;
    }
    frames
}

struct ProxyAddress {
    host_header: String,
    socket_address: String,
}

fn parse_proxy_address(host: &str) -> Result<ProxyAddress, String> {
    let host = host
        .strip_prefix("http://")
        .unwrap_or(host)
        .trim_end_matches('/');
    if host.is_empty()
        || host.contains(['/', '?', '#'])
        || host
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("invalid AVDECC Proxy host".to_string());
    }
    if let Some(rest) = host.strip_prefix('[') {
        let closing = rest
            .find(']')
            .ok_or("unterminated bracketed IPv6 AVDECC Proxy host")?;
        let address = &rest[..closing];
        address
            .parse::<Ipv6Addr>()
            .map_err(|_| "invalid bracketed IPv6 AVDECC Proxy host")?;
        let suffix = &rest[closing + 1..];
        let port = if suffix.is_empty() {
            17221
        } else {
            suffix
                .strip_prefix(':')
                .ok_or("invalid bracketed IPv6 AVDECC Proxy host")?
                .parse::<u16>()
                .map_err(|_| "invalid AVDECC Proxy port")?
        };
        return Ok(ProxyAddress {
            host_header: format!("[{address}]:{port}"),
            socket_address: format!("[{address}]:{port}"),
        });
    }
    if host.parse::<Ipv6Addr>().is_ok() {
        return Ok(ProxyAddress {
            host_header: format!("[{host}]:17221"),
            socket_address: format!("[{host}]:17221"),
        });
    }
    if let Some((name, port)) = host.rsplit_once(':') {
        if name.is_empty() || port.parse::<u16>().is_err() {
            return Err("invalid AVDECC Proxy host; expected host or host:port".to_string());
        }
        return Ok(ProxyAddress {
            host_header: host.to_string(),
            socket_address: host.to_string(),
        });
    }
    Ok(ProxyAddress {
        host_header: format!("{host}:17221"),
        socket_address: format!("{host}:17221"),
    })
}

fn validate_proxy_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || !path.starts_with('/')
        || path.starts_with("//")
        || path
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("AVDECC Proxy path must be a whitespace-free origin path".to_string());
    }
    Ok(())
}

fn connect_with_timeout(address: &str, timeout: Duration) -> Result<TcpStream, String> {
    let addresses = address
        .to_socket_addrs()
        .map_err(|error| format!("resolve AVDECC Proxy {address} failed: {error}"))?
        .collect::<Vec<_>>();
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error.ok_or("AVDECC Proxy host resolved to no addresses")?;
    Err(format!("connect AVDECC Proxy {address} failed: {error}"))
}

fn app_frame_json(frame: &AppFrame) -> String {
    let address = frame
        .address
        .iter()
        .map(|octet| format!("{octet:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    let payload_preview = hex_preview(&frame.payload, 48);
    format!(
        "{{\"version\":{},\"message_type\":\"{}\",\"address\":\"{}\",\"payload_bytes\":{},\"payload_preview\":\"{}\"}}",
        frame.version,
        app_message_type_name(frame.message_type),
        address,
        frame.payload.len(),
        payload_preview
    )
}

fn app_message_type_name(message_type: u8) -> &'static str {
    match message_type {
        APP_NOP => "nop",
        APP_ENTITY_GUID_REQUEST => "entity_guid_request",
        APP_ENTITY_GUID_RESPONSE => "entity_guid_response",
        APP_LINK_UP => "link_up",
        APP_LINK_DOWN => "link_down",
        APP_AVDECC_FROM_APS => "avdecc_from_aps",
        APP_AVDECC_FROM_APC => "avdecc_from_apc",
        0xff => "vendor",
        _ => "unknown",
    }
}

#[cfg(test)]
#[path = "avdecc_tests.rs"]
mod tests;
