use std::collections::HashMap;
#[cfg(not(target_os = "windows"))]
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::device::{datastore_write_request, json_escape, percent_decode, DeviceClient};
use crate::discovery::{browser_control_hosts, discover_avdecc, DiscoveryResult};
use crate::probe::{probe_device, probe_result_json};
use crate::ui;

const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_REQUEST_HEADER_BYTES: usize = 32 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

#[cfg(target_os = "windows")]
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

#[cfg(target_os = "windows")]
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut core::ffi::c_void,
        buffer: *mut u8,
        buffer_length: u32,
        flags: u32,
    ) -> i32;
}

enum ServerScope {
    Configured(String),
    Discovered(Vec<DiscoveryResult>),
}

pub(crate) fn serve(
    default_host: Option<&str>,
    listen: &str,
    timeout: Duration,
) -> Result<(), String> {
    let listen_address = listen
        .parse::<SocketAddr>()
        .map_err(|_| "--listen must be a numeric loopback address, such as 127.0.0.1:8480")?;
    if !listen_address.ip().is_loopback() {
        return Err("the browser control server may only listen on a loopback address".to_string());
    }
    let scope = match default_host {
        Some(host) => ServerScope::Configured(host.to_string()),
        None => ServerScope::Discovered(discover_avdecc(timeout)?),
    };
    let listener = TcpListener::bind(listen_address)
        .map_err(|error| format!("listen on {listen_address} failed: {error}"))?;
    let expected_origin = listener_origin(&listener)?;
    let session_token = new_session_token()?;
    println!("cuemix-848 UI: {expected_origin}");
    match &scope {
        ServerScope::Configured(host) => println!("default device: {host}"),
        ServerScope::Discovered(devices) => {
            println!("discovered devices: {}", devices.len());
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_browser_request(
                    stream,
                    &scope,
                    &expected_origin,
                    &session_token,
                    timeout,
                ) {
                    eprintln!("request failed: {error}");
                }
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    Ok(())
}

fn listener_origin(listener: &TcpListener) -> Result<String, String> {
    let address = listener
        .local_addr()
        .map_err(|error| format!("read bound address failed: {error}"))?;
    Ok(origin_for_address(address))
}

fn origin_for_address(address: SocketAddr) -> String {
    format!("http://{address}")
}

fn new_session_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fill_session_entropy(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(not(target_os = "windows"))]
fn fill_session_entropy(bytes: &mut [u8]) -> Result<(), String> {
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|error| format!("read session entropy failed: {error}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn fill_session_entropy(bytes: &mut [u8]) -> Result<(), String> {
    let buffer_length = u32::try_from(bytes.len())
        .map_err(|_| "session entropy buffer is too large for Windows RNG".to_string())?;
    let status = unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            bytes.as_mut_ptr(),
            buffer_length,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "generate session entropy failed: BCryptGenRandom returned 0x{:08x}",
            status as u32
        ))
    }
}

fn handle_browser_request(
    mut stream: TcpStream,
    scope: &ServerScope,
    expected_origin: &str,
    session_token: &str,
    timeout: Duration,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("set browser read timeout failed: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("set browser write timeout failed: {error}"))?;
    let request = read_browser_request(&stream, timeout);
    let response = match request {
        Ok(request) => route_browser_request(
            &request.method,
            &request.target,
            &request.body,
            request.origin.as_deref(),
            scope,
            expected_origin,
            session_token,
            timeout,
        ),
        Err(error) => json_error(400, &error),
    };
    write_browser_response(&mut stream, response)
}

struct BrowserRequest {
    method: String,
    target: String,
    body: String,
    origin: Option<String>,
}

fn read_browser_request(stream: &TcpStream, timeout: Duration) -> Result<BrowserRequest, String> {
    let started = Instant::now();
    let mut reader = io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let first_line = read_limited_line(
        &mut reader,
        MAX_REQUEST_LINE_BYTES,
        "request line",
        started,
        timeout,
    )?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().ok_or("missing HTTP method")?.to_string();
    let target = parts.next().ok_or("missing request target")?.to_string();
    if parts.next().is_none() || !target.starts_with('/') {
        return Err("invalid browser request line".to_string());
    }

    let mut header_bytes: usize = 0;
    let mut content_length = None;
    let mut origin = None;
    loop {
        let line = read_limited_line(
            &mut reader,
            MAX_REQUEST_LINE_BYTES,
            "request header",
            started,
            timeout,
        )?;
        header_bytes = header_bytes
            .checked_add(line.len())
            .ok_or("browser request header size overflow")?;
        if header_bytes > MAX_REQUEST_HEADER_BYTES {
            return Err("browser request headers are too large".to_string());
        }
        if line == "\r\n" || line.is_empty() {
            break;
        }
        let (name, value) = line
            .trim_end_matches(['\r', '\n'])
            .split_once(':')
            .ok_or("invalid browser request header")?;
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("multiple content-length headers are not allowed".to_string());
            }
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "invalid content-length")?;
            if length > MAX_REQUEST_BODY_BYTES {
                return Err("browser request body is too large".to_string());
            }
            content_length = Some(length);
        }
        if name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim().to_string());
        }
    }

    let body = read_browser_body(&mut reader, content_length.unwrap_or(0), started, timeout)?;
    Ok(BrowserRequest {
        method,
        target,
        body,
        origin,
    })
}

fn read_limited_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
    description: &str,
    started: Instant,
    timeout: Duration,
) -> Result<String, String> {
    let mut line = Vec::new();
    loop {
        check_request_deadline(started, timeout)?;
        let (take, found_newline) = {
            let buffer = reader
                .fill_buf()
                .map_err(|error| format!("read {description} failed: {error}"))?;
            check_request_deadline(started, timeout)?;
            if buffer.is_empty() {
                return Err(format!("unexpected end of {description}"));
            }
            let found_newline = buffer.iter().position(|byte| *byte == b'\n');
            let take = found_newline.map_or(buffer.len(), |index| index + 1);
            if line
                .len()
                .checked_add(take)
                .filter(|length| *length <= max_bytes)
                .is_none()
            {
                return Err(format!("{description} is too large"));
            }
            line.extend_from_slice(&buffer[..take]);
            (take, found_newline.is_some())
        };
        reader.consume(take);
        if found_newline {
            return String::from_utf8(line).map_err(|_| format!("{description} must be UTF-8"));
        }
    }
}

fn read_browser_body(
    reader: &mut impl Read,
    length: usize,
    started: Instant,
    timeout: Duration,
) -> Result<String, String> {
    let mut body = vec![0u8; length];
    let mut cursor = 0;
    while cursor < body.len() {
        check_request_deadline(started, timeout)?;
        let count = reader
            .read(&mut body[cursor..])
            .map_err(|error| format!("read browser request body failed: {error}"))?;
        check_request_deadline(started, timeout)?;
        if count == 0 {
            return Err("unexpected end of browser request body".to_string());
        }
        cursor += count;
    }
    String::from_utf8(body).map_err(|_| "browser request body must be UTF-8".to_string())
}

fn check_request_deadline(started: Instant, timeout: Duration) -> Result<(), String> {
    if started.elapsed() > timeout {
        Err("browser request exceeded overall timeout".to_string())
    } else {
        Ok(())
    }
}

struct BrowserResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

#[allow(clippy::too_many_arguments)]
fn route_browser_request(
    method: &str,
    target: &str,
    body: &str,
    origin: Option<&str>,
    scope: &ServerScope,
    expected_origin: &str,
    session_token: &str,
    timeout: Duration,
) -> BrowserResponse {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        ("GET", "/") => {
            let params = parse_query(query);
            match scope {
                ServerScope::Discovered(devices) if !params.contains_key("host") => {
                    BrowserResponse {
                        status: 200,
                        content_type: "text/html; charset=utf-8",
                        body: ui::render_discovery(devices),
                    }
                }
                _ => match allowed_host(&params, scope) {
                    Ok(host) => BrowserResponse {
                        status: 200,
                        content_type: "text/html; charset=utf-8",
                        body: ui::render(&host, session_token),
                    },
                    Err(error) => json_error(400, &error),
                },
            }
        }
        ("GET", "/api/probe") => {
            let params = parse_query(query);
            match allowed_host(&params, scope).and_then(|host| DeviceClient::new(&host, timeout)) {
                Ok(client) => {
                    let results = probe_device(&client);
                    let body = format!(
                        "[{}]",
                        results
                            .iter()
                            .map(probe_result_json)
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    json_response(200, body)
                }
                Err(error) => json_error(400, &error),
            }
        }
        ("GET", "/api/get") => {
            let params = parse_query(query);
            proxy_get_or_error(&params, scope, timeout)
        }
        ("POST", "/api/set") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_set_or_error(&params, scope, timeout)
        }
        _ => BrowserResponse {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: "not found".to_string(),
        },
    }
}

fn is_authorized(
    origin: Option<&str>,
    token: Option<&String>,
    expected_origin: &str,
    session_token: &str,
) -> bool {
    origin == Some(expected_origin)
        && token.is_some_and(|token| constant_time_eq(token, session_token))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn allowed_host(params: &HashMap<String, String>, scope: &ServerScope) -> Result<String, String> {
    match scope {
        ServerScope::Configured(default_host) => match params.get("host") {
            None => Ok(default_host.clone()),
            Some(host) if host == default_host => Ok(default_host.clone()),
            Some(_) => Err("this server is limited to its configured device host".to_string()),
        },
        ServerScope::Discovered(devices) => {
            let host = params
                .get("host")
                .ok_or("select a discovered device before using the control API")?;
            if devices
                .iter()
                .flat_map(browser_control_hosts)
                .any(|candidate| candidate == *host)
            {
                Ok(host.clone())
            } else {
                Err("this server is limited to addresses discovered at startup".to_string())
            }
        }
    }
}

fn proxy_get_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    timeout: Duration,
) -> BrowserResponse {
    let Some(path) = params.get("path") else {
        return json_error(400, "missing path");
    };
    match allowed_host(params, scope) {
        Ok(host) => proxy_request(&host, "GET", path, None, timeout),
        Err(error) => json_error(400, &error),
    }
}

fn proxy_set_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    timeout: Duration,
) -> BrowserResponse {
    let Some(path) = params.get("path") else {
        return json_error(400, "missing path");
    };
    let Some(value) = params.get("value") else {
        return json_error(400, "missing value");
    };
    let method = params.get("method").map(String::as_str).unwrap_or("POST");
    if method != "POST" && method != "PATCH" {
        return json_error(400, "method must be POST or PATCH");
    }
    let host = match allowed_host(params, scope) {
        Ok(host) => host,
        Err(error) => return json_error(400, &error),
    };
    let (request_path, body) = match datastore_write_request(path, value) {
        Ok(request) => request,
        Err(error) => return json_error(400, &error),
    };
    proxy_request(&host, method, &request_path, Some(&body), timeout)
}

fn proxy_request(
    host: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> BrowserResponse {
    match DeviceClient::new(host, timeout).and_then(|client| client.request(method, path, body)) {
        Ok(response) => json_response(
            200,
            format!(
                "{{\"status\":{},\"reason\":\"{}\",\"body\":\"{}\"}}",
                response.status,
                json_escape(&response.reason),
                json_escape(&response.body)
            ),
        ),
        Err(error) => json_error(502, &error),
    }
}

fn json_response(status: u16, body: String) -> BrowserResponse {
    BrowserResponse {
        status,
        content_type: "application/json; charset=utf-8",
        body,
    }
}

fn json_error(status: u16, message: &str) -> BrowserResponse {
    json_response(
        status,
        format!("{{\"error\":\"{}\"}}", json_escape(message)),
    )
}

fn write_browser_response(stream: &mut TcpStream, response: BrowserResponse) -> Result<(), String> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(response.body.as_bytes()))
        .map_err(|error| format!("write browser response failed: {error}"))
}

fn parse_query(input: &str) -> HashMap<String, String> {
    let mut output = HashMap::new();
    for pair in input.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        output.insert(percent_decode(key), percent_decode(value));
    }
    output
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
