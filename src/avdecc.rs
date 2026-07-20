use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::device::{parse_http_response, HttpResponse};

#[path = "avdecc_format.rs"]
mod avdecc_format;

use avdecc_format::hex_preview;

#[path = "avdecc_transport.rs"]
mod avdecc_transport;

use avdecc_transport::{connect_with_timeout, parse_proxy_address, validate_proxy_path};

#[path = "avdecc_aem.rs"]
mod avdecc_aem;

#[path = "avdecc_descriptor.rs"]
mod avdecc_descriptor;

#[path = "avdecc_probe.rs"]
mod avdecc_probe;

pub(crate) use avdecc_probe::{probe, write_probe_result, DescriptorRead};

// IEEE 1722.1-2013 Annex C APPDU: version, type, payload length, EUI-48,
// then a reserved/status u16 before the payload.
const APP_HEADER_LEN: usize = 12;
const APP_MAX_PAYLOAD_LEN: usize = 1490;
const CONNECT_HEADER_LIMIT: usize = 32 * 1024;
const INITIAL_FRAME_WAIT: Duration = Duration::from_millis(250);
const INITIAL_DATA_LIMIT: usize = 8 * 1024;

const APP_NOP: u8 = 0x00;
const APP_ENTITY_ID_REQUEST: u8 = 0x01;
const APP_ENTITY_ID_RESPONSE: u8 = 0x02;
const APP_LINK_UP: u8 = 0x03;
const APP_LINK_DOWN: u8 = 0x04;
const APP_AVDECC_FROM_APS: u8 = 0x05;
const APP_AVDECC_FROM_APC: u8 = 0x06;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppFrame {
    version: u8,
    message_type: u8,
    address: [u8; 6],
    reserved: u16,
    payload: Vec<u8>,
}

impl AppFrame {
    fn encode(&self) -> Result<Vec<u8>, String> {
        if self.payload.len() > APP_MAX_PAYLOAD_LEN {
            return Err(format!(
                "AVDECC Proxy payload exceeds {APP_MAX_PAYLOAD_LEN} byte limit"
            ));
        }
        let mut bytes = Vec::with_capacity(APP_HEADER_LEN + self.payload.len());
        bytes.push(self.version);
        bytes.push(self.message_type);
        bytes.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&self.address);
        bytes.extend_from_slice(&self.reserved.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
}

#[derive(Default)]
struct EntityIdResult {
    entity_id: Option<u64>,
    reserved: Option<u16>,
    frames: Vec<AppFrame>,
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

    fn read_available_for(&mut self, wait: Duration, preserve: bool) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let mut data = if preserve {
            self.buffered.clone()
        } else {
            std::mem::take(&mut self.buffered)
        };
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
                    append_preview_bytes(&mut data, &mut self.buffered, &buffer[..count], preserve)
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

    fn request_entity_id(
        &mut self,
        primary_mac: [u8; 6],
        timeout: Duration,
    ) -> Result<EntityIdResult, String> {
        // This allocates an ephemeral controller identity in the proxy. It is
        // not an AECP command and cannot modify the attached AVDECC entity.
        let request = AppFrame {
            version: 0,
            message_type: APP_ENTITY_ID_REQUEST,
            address: primary_mac,
            reserved: 0,
            payload: vec![0; 8],
        };
        self.stream
            .write_all(&request.encode()?)
            .map_err(|error| format!("write AVDECC Proxy entity ID request failed: {error}"))?;

        let deadline = Instant::now() + timeout;
        let mut frames = Vec::new();
        loop {
            let frame = self
                .read_frame_until(deadline)?
                .ok_or("timed out waiting for AVDECC Proxy entity ID response")?;
            let is_response = is_entity_id_response(&frame, primary_mac);
            frames.push(frame.clone());
            if is_response {
                let entity_id: [u8; 8] = frame
                    .payload
                    .as_slice()
                    .try_into()
                    .map_err(|_| "invalid AVDECC Proxy entity ID response length")?;
                return Ok(EntityIdResult {
                    entity_id: Some(u64::from_be_bytes(entity_id)),
                    reserved: Some(frame.reserved),
                    frames,
                });
            }
        }
    }

    fn read_frame_until(&mut self, deadline: Instant) -> Result<Option<AppFrame>, String> {
        let Some(header) = self.read_exact_until(APP_HEADER_LEN, deadline)? else {
            return Ok(None);
        };
        if header[0] != 0 {
            return Err(format!(
                "unsupported AVDECC Proxy frame version {}",
                header[0]
            ));
        }
        let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        if payload_len > APP_MAX_PAYLOAD_LEN {
            return Err(format!(
                "AVDECC Proxy frame payload exceeds {APP_MAX_PAYLOAD_LEN} byte limit"
            ));
        }
        let payload = self
            .read_exact_until(payload_len, deadline)?
            .ok_or("timed out while reading AVDECC Proxy frame payload")?;
        decode_app_frame(&header, payload).map(Some)
    }

    fn read_exact_until(
        &mut self,
        length: usize,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut output = Vec::with_capacity(length);
        while output.len() < length {
            if !self.buffered.is_empty() {
                let take = (length - output.len()).min(self.buffered.len());
                output.extend(self.buffered.drain(..take));
                continue;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return if output.is_empty() {
                    Ok(None)
                } else {
                    Err(timeout_error(&output, length))
                };
            };
            if remaining.is_zero() {
                return if output.is_empty() {
                    Ok(None)
                } else {
                    Err(timeout_error(&output, length))
                };
            }
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
            let mut buffer = [0u8; 1536];
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err("AVDECC Proxy closed the tunnel".to_string()),
                Ok(count) => self.buffered.extend_from_slice(&buffer[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return if output.is_empty() {
                        Ok(None)
                    } else {
                        Err(timeout_error(&output, length))
                    };
                }
                Err(error) => return Err(format!("read AVDECC Proxy tunnel failed: {error}")),
            }
        }
        Ok(Some(output))
    }
}

fn append_preview_bytes(data: &mut Vec<u8>, buffered: &mut Vec<u8>, bytes: &[u8], preserve: bool) {
    let remaining_capacity = INITIAL_DATA_LIMIT.saturating_sub(data.len());
    data.extend_from_slice(&bytes[..bytes.len().min(remaining_capacity)]);
    if preserve {
        buffered.extend_from_slice(bytes);
    }
}

fn is_entity_id_response(frame: &AppFrame, primary_mac: [u8; 6]) -> bool {
    frame.version == 0
        && frame.message_type == APP_ENTITY_ID_RESPONSE
        && frame.address == primary_mac
}

fn timeout_error(received: &[u8], expected: usize) -> String {
    if received.is_empty() {
        "timed out waiting for AVDECC Proxy frame".to_string()
    } else {
        format!(
            "timed out while reading AVDECC Proxy frame: received {} of {expected} bytes ({})",
            received.len(),
            hex_preview(received, 32)
        )
    }
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
    if !is_known_v0_message_type(header[1]) {
        return Err("invalid AVDECC Proxy frame message type".to_string());
    }
    let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
    if payload.len() != payload_len {
        return Err("invalid AVDECC Proxy frame payload length".to_string());
    }
    let address: [u8; 6] = header[4..10]
        .try_into()
        .map_err(|_| "invalid AVDECC Proxy frame address")?;
    let reserved = u16::from_be_bytes([header[10], header[11]]);
    Ok(AppFrame {
        version: header[0],
        message_type: header[1],
        address,
        reserved,
        payload,
    })
}

fn is_known_v0_message_type(message_type: u8) -> bool {
    matches!(
        message_type,
        APP_NOP
            | APP_ENTITY_ID_REQUEST
            | APP_ENTITY_ID_RESPONSE
            | APP_LINK_UP
            | APP_LINK_DOWN
            | APP_AVDECC_FROM_APS
            | APP_AVDECC_FROM_APC
            | 0xff
    )
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

#[cfg(test)]
#[path = "avdecc_tests.rs"]
mod tests;
