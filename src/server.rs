use std::collections::HashMap;
#[cfg(not(target_os = "windows"))]
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::avdecc::{
    parse_changes, set_headphone_trim, set_line_input_phase, set_line_output_trim, set_mixer_fader,
    start_mixer_meter_worker, write_console, HeadphoneOutput, LineInput, LineOutput, MeterPath,
    MixerFader, MixerLevel, MixerMeterFeed, MixerMeterRecord, MixerMeters, OutputInventory,
    OutputTrim, VendorSnapshot, VendorSnapshotRequest,
};
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

#[derive(Default)]
struct MeterHub {
    workers: Mutex<HashMap<String, MeterWorker>>,
}

struct MeterWorker {
    stop_sender: mpsc::Sender<mpsc::Sender<()>>,
    state_sender: mpsc::Sender<VendorSnapshotRequest>,
    pending_stop: Option<mpsc::Receiver<()>>,
    meters: Arc<MixerMeterFeed>,
}

impl MeterHub {
    fn reap_stopped(workers: &mut HashMap<String, MeterWorker>, host: &str) -> Result<(), String> {
        if let Some(receiver) = workers
            .get(host)
            .and_then(|worker| worker.pending_stop.as_ref())
        {
            match receiver.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                    workers.remove(host);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    return Err("device session is closing; retry refresh".into())
                }
            }
        }
        Ok(())
    }

    fn existing_snapshot(&self, host: &str) -> Result<Option<MixerMeters>, String> {
        self.existing_feed(host)?
            .map(|meters| meters.snapshot().map(|snapshot| snapshot.meters))
            .transpose()
    }

    fn existing_feed(&self, host: &str) -> Result<Option<Arc<MixerMeterFeed>>, String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "meter worker registry is unavailable".to_string())?;
        Self::reap_stopped(&mut workers, host)?;
        Ok(workers.get(host).map(|worker| Arc::clone(&worker.meters)))
    }

    fn start(
        &self,
        host: &str,
        target_entity_id: u64,
        timeout: Duration,
    ) -> Result<Arc<MixerMeterFeed>, String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "meter worker registry is unavailable".to_string())?;
        Self::reap_stopped(&mut workers, host)?;
        let worker = workers.entry(host.to_string()).or_insert_with(|| {
            let worker = start_mixer_meter_worker(host.to_string(), target_entity_id, timeout);
            MeterWorker {
                stop_sender: worker.stop_sender,
                state_sender: worker.state_sender,
                meters: worker.meters,
                pending_stop: None,
            }
        });
        Ok(Arc::clone(&worker.meters))
    }

    fn start_and_snapshot(
        &self,
        host: &str,
        target_entity_id: u64,
        timeout: Duration,
    ) -> Result<MixerMeters, String> {
        self.start(host, target_entity_id, timeout)?
            .snapshot()
            .map(|snapshot| snapshot.meters)
    }

    fn stop(&self, host: &str, timeout: Duration) -> Result<(), String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "meter worker registry is unavailable".to_string())?;
        let Some(worker) = workers.get_mut(host) else {
            return Ok(());
        };
        if worker.pending_stop.is_none() {
            let (sender, receiver) = mpsc::channel();
            if worker.stop_sender.send(sender).is_err() {
                workers.remove(host);
                return Ok(());
            }
            worker.pending_stop = Some(receiver);
        }
        worker
            .pending_stop
            .as_ref()
            .unwrap()
            .recv_timeout(timeout)
            .map_err(|_| "meter worker did not close its proxy session in time".to_string())?;
        workers.remove(host);
        Ok(())
    }

    fn read_state(
        &self,
        host: &str,
        target: u64,
        timeout: Duration,
    ) -> Result<VendorSnapshot, String> {
        self.start(host, target, timeout)?;
        let sender = {
            let workers = self
                .workers
                .lock()
                .map_err(|_| "meter worker registry is unavailable")?;
            let worker = workers.get(host).ok_or("meter worker is unavailable")?;
            if worker.pending_stop.is_some() {
                return Err("device session is closing; retry refresh".into());
            }
            worker.state_sender.clone()
        };
        let (reply, receiver) = mpsc::channel();
        sender
            .send(VendorSnapshotRequest {
                deadline: Instant::now() + timeout,
                reply,
            })
            .map_err(|_| "device session is unavailable".to_string())?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| "device refresh timed out".to_string())?
    }
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
    let meter_hub = MeterHub::default();
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
                    &meter_hub,
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
    meter_hub: &MeterHub,
    timeout: Duration,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("set browser read timeout failed: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("set browser write timeout failed: {error}"))?;
    let request = match read_browser_request(&stream, timeout) {
        Ok(request) => request,
        Err(error) => return write_browser_response(&mut stream, json_error(400, &error)),
    };
    if request.method == "GET"
        && request
            .target
            .split_once('?')
            .map_or(request.target.as_str(), |target| target.0)
            == "/api/mixer/meters/events"
    {
        return start_mixer_meter_event_stream(stream, &request.target, scope, meter_hub, timeout);
    }
    let response = route_browser_request(
        &request.method,
        &request.target,
        &request.body,
        request.origin.as_deref(),
        scope,
        expected_origin,
        session_token,
        meter_hub,
        timeout,
    );
    write_browser_response(&mut stream, response)
}

fn start_mixer_meter_event_stream(
    mut stream: TcpStream,
    target: &str,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> Result<(), String> {
    let query = target.split_once('?').map_or("", |target| target.1);
    let params = parse_query(query);
    let host = match allowed_host(&params, scope) {
        Ok(host) => host,
        Err(error) => return write_browser_response(&mut stream, json_error(400, &error)),
    };
    let feed = match meter_hub.existing_feed(&host) {
        Ok(Some(feed)) => feed,
        Ok(None) => {
            let target_entity_id = match DeviceClient::new(&host, timeout)
                .and_then(|client| client.request("GET", "/datastore", None))
                .and_then(|response| datastore_entity_id(&response.body))
            {
                Ok(entity_id) => entity_id,
                Err(error) => {
                    return write_browser_response(&mut stream, json_error(502, &error));
                }
            };
            match meter_hub.start(&host, target_entity_id, timeout) {
                Ok(feed) => feed,
                Err(error) => {
                    return write_browser_response(&mut stream, json_error(502, &error));
                }
            }
        }
        Err(error) => return write_browser_response(&mut stream, json_error(502, &error)),
    };
    stream
        .set_nodelay(true)
        .map_err(|error| format!("configure meter event stream failed: {error}"))?;
    write_mixer_meter_event_headers(&mut stream)?;
    thread::spawn(move || stream_mixer_meter_events(stream, feed));
    Ok(())
}

fn write_mixer_meter_event_headers(stream: &mut impl Write) -> Result<(), String> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\nretry: 100\n\n",
        )
        .map_err(|error| format!("write meter event stream headers failed: {error}"))
}

fn stream_mixer_meter_events(mut stream: TcpStream, feed: Arc<MixerMeterFeed>) {
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
    let Ok(mut update) = feed.snapshot() else {
        return;
    };
    loop {
        if update.closed {
            return;
        }
        if !update.meters.records.is_empty()
            && write_mixer_meter_event(&mut stream, update.revision, &update.meters).is_err()
        {
            return;
        }
        match feed.wait_after(update.revision, KEEPALIVE_INTERVAL) {
            Ok(Some(next)) => update = next,
            Ok(None) => {
                if stream.write_all(b": keepalive\n\n").is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

fn write_mixer_meter_event(
    stream: &mut impl Write,
    revision: u64,
    meters: &MixerMeters,
) -> Result<(), String> {
    write!(
        stream,
        "id: {revision}\nevent: meters\ndata: {}\n\n",
        mixer_meters_json(meters)
    )
    .map_err(|error| format!("write meter event failed: {error}"))
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
    meter_hub: &MeterHub,
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
        ("GET", "/api/console") => {
            let params = parse_query(query);
            let (host, target) = match vendor_target(&params, scope, timeout) {
                Ok(target) => target,
                Err(error) => return error,
            };
            match meter_hub
                .read_state(&host, target, timeout)
                .and_then(VendorSnapshot::console)
            {
                Ok(state) => json_response(200, state.json()),
                Err(error) => json_error(502, &error),
            }
        }
        ("POST", "/api/console/changes") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            let changes = match params
                .get("changes")
                .ok_or_else(|| "missing changes".to_string())
                .and_then(|s| parse_changes(s))
            {
                Ok(changes) => changes,
                Err(error) => return json_error(400, &error),
            };
            let (host, target) = match vendor_target(&params, scope, timeout) {
                Ok(target) => target,
                Err(error) => return error,
            };
            if let Err(error) = meter_hub.stop(&host, timeout) {
                return json_error(502, &error);
            }
            match write_console(&host, target, &changes, timeout) {
                Ok(applied) => json_response(200, format!(r#"{{"acknowledged":{applied}}}"#)),
                Err(error) => json_response(if error.conflict { 409 } else { 502 }, error.json()),
            }
        }
        ("GET", "/api/mixer/meters") => {
            let params = parse_query(query);
            proxy_mixer_meters_or_error(&params, scope, meter_hub, timeout)
        }
        ("GET", "/api/inputs/lines") => {
            let params = parse_query(query);
            proxy_line_inputs_or_error(&params, scope, meter_hub, timeout)
        }
        ("GET", "/api/outputs") => {
            let params = parse_query(query);
            proxy_outputs_or_error(&params, scope, meter_hub, timeout)
        }
        ("GET", "/api/outputs/headphones") => {
            let params = parse_query(query);
            proxy_headphones_or_error(&params, scope, meter_hub, timeout)
        }
        ("POST", "/api/set") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_set_or_error(&params, scope, timeout)
        }
        ("POST", "/api/mixer/fader") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_mixer_fader_or_error(&params, scope, meter_hub, timeout)
        }
        ("POST", "/api/inputs/line-phase") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_line_input_phase_or_error(&params, scope, meter_hub, timeout)
        }
        ("POST", "/api/outputs/headphone-trim") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_headphone_trim_or_error(&params, scope, meter_hub, timeout)
        }
        ("POST", "/api/outputs/line-trim") => {
            let mut params = parse_query(query);
            params.extend(parse_query(body));
            if !is_authorized(origin, params.get("token"), expected_origin, session_token) {
                return json_error(403, "invalid origin or session token");
            }
            proxy_line_output_trim_or_error(&params, scope, meter_hub, timeout)
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

fn proxy_mixer_fader_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let Some(bus) = params.get("bus") else {
        return json_error(400, "missing mixer bus");
    };
    let Some(source) = params.get("source") else {
        return json_error(400, "missing mixer source");
    };
    let Some(level) = params.get("level") else {
        return json_error(400, "missing mixer level");
    };
    let fader = match MixerFader::parse(bus, source) {
        Ok(fader) => fader,
        Err(error) => return json_error(400, &error),
    };
    let level = match MixerLevel::parse(level) {
        Ok(level) => level,
        Err(error) => return json_error(400, &error),
    };
    let host = match allowed_host(params, scope) {
        Ok(host) => host,
        Err(error) => return json_error(400, &error),
    };
    let target_entity_id = match DeviceClient::new(&host, timeout)
        .and_then(|client| client.request("GET", "/datastore", None))
        .and_then(|response| datastore_entity_id(&response.body))
    {
        Ok(entity_id) => entity_id,
        Err(error) => return json_error(502, &error),
    };
    // Meter polling owns its own vendor session. Close it before opening the
    // short-lived fader session so the 848 never sees competing controllers
    // from this local server.
    if let Err(error) = meter_hub.stop(&host, timeout) {
        return json_error(502, &error);
    }
    match set_mixer_fader(&host, target_entity_id, fader, level, timeout) {
        Ok(()) => json_response(
            200,
            "{\"status\":200,\"body\":\"fader acknowledged\"}".to_string(),
        ),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_mixer_meters_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let host = match allowed_host(params, scope) {
        Ok(host) => host,
        Err(error) => return json_error(400, &error),
    };
    match meter_hub.existing_snapshot(&host) {
        Ok(Some(snapshot)) => return json_response(200, mixer_meters_json(&snapshot)),
        Ok(None) => {}
        Err(error) => return json_error(502, &error),
    }
    let target_entity_id = match DeviceClient::new(&host, timeout)
        .and_then(|client| client.request("GET", "/datastore", None))
        .and_then(|response| datastore_entity_id(&response.body))
    {
        Ok(entity_id) => entity_id,
        Err(error) => return json_error(502, &error),
    };
    match meter_hub.start_and_snapshot(&host, target_entity_id, timeout) {
        Ok(snapshot) => json_response(200, mixer_meters_json(&snapshot)),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_outputs_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    match meter_hub
        .read_state(&host, target_entity_id, timeout)
        .and_then(|state| state.outputs())
    {
        Ok(inventory) => json_response(200, output_inventory_json(&inventory)),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_headphones_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    match meter_hub
        .read_state(&host, target_entity_id, timeout)
        .and_then(|state| state.outputs())
    {
        Ok(inventory) => json_response(200, headphone_outputs_json(&inventory.headphone_outputs)),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_headphone_trim_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let output_index = match params
        .get("output")
        .ok_or("missing headphone output")
        .and_then(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "headphone output must be a zero-based integer")
        }) {
        Ok(index) => index,
        Err(error) => return json_error(400, error),
    };
    let trim = match params
        .get("trim_db")
        .ok_or_else(|| "missing headphone trim".to_string())
        .and_then(|value| OutputTrim::parse(value))
    {
        Ok(trim) => trim,
        Err(error) => return json_error(400, &error),
    };
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    if let Err(error) = meter_hub.stop(&host, timeout) {
        return json_error(502, &error);
    }
    match set_headphone_trim(&host, target_entity_id, output_index, trim, timeout) {
        Ok(()) => json_response(
            200,
            format!(
                "{{\"status\":200,\"body\":\"Phones {} trim acknowledged\"}}",
                output_index + 1
            ),
        ),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_line_output_trim_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let output_index = match params
        .get("output")
        .ok_or("missing line output")
        .and_then(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "line output must be a zero-based integer")
        }) {
        Ok(index) => index,
        Err(error) => return json_error(400, error),
    };
    let trim = match params
        .get("trim_db")
        .ok_or_else(|| "missing line-output trim".to_string())
        .and_then(|value| OutputTrim::parse(value))
    {
        Ok(trim) => trim,
        Err(error) => return json_error(400, &error),
    };
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    if let Err(error) = meter_hub.stop(&host, timeout) {
        return json_error(502, &error);
    }
    match set_line_output_trim(&host, target_entity_id, output_index, trim, timeout) {
        Ok(()) => json_response(
            200,
            format!(
                "{{\"status\":200,\"body\":\"Line Out {} trim acknowledged\"}}",
                output_index + 1
            ),
        ),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_line_inputs_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    match meter_hub
        .read_state(&host, target_entity_id, timeout)
        .and_then(|state| state.line_inputs())
    {
        Ok(inputs) => json_response(200, line_inputs_json(&inputs)),
        Err(error) => json_error(502, &error),
    }
}

fn proxy_line_input_phase_or_error(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    meter_hub: &MeterHub,
    timeout: Duration,
) -> BrowserResponse {
    let input_index = match line_input_index(params) {
        Ok(index) => index,
        Err(error) => return json_error(400, error),
    };
    let phase_inverted = match params.get("enabled").map(String::as_str) {
        Some("0") => false,
        Some("1") => true,
        Some(_) => return json_error(400, "line-input phase must be 0 or 1"),
        None => return json_error(400, "missing line-input phase"),
    };
    let (host, target_entity_id) = match vendor_target(params, scope, timeout) {
        Ok(target) => target,
        Err(response) => return response,
    };
    if let Err(error) = meter_hub.stop(&host, timeout) {
        return json_error(502, &error);
    }
    match set_line_input_phase(
        &host,
        target_entity_id,
        input_index,
        phase_inverted,
        timeout,
    ) {
        Ok(()) => json_response(
            200,
            format!(
                "{{\"status\":200,\"body\":\"Line In {} polarity acknowledged\"}}",
                input_index + 5
            ),
        ),
        Err(error) => json_error(502, &error),
    }
}

fn line_input_index(params: &HashMap<String, String>) -> Result<usize, &'static str> {
    params
        .get("input")
        .ok_or("missing line input")?
        .parse::<usize>()
        .map_err(|_| "line input must be a zero-based integer")
}

fn vendor_target(
    params: &HashMap<String, String>,
    scope: &ServerScope,
    timeout: Duration,
) -> Result<(String, u64), BrowserResponse> {
    let host = allowed_host(params, scope).map_err(|error| json_error(400, &error))?;
    let target_entity_id = DeviceClient::new(&host, timeout)
        .and_then(|client| client.request("GET", "/datastore", None))
        .and_then(|response| datastore_entity_id(&response.body))
        .map_err(|error| json_error(502, &error))?;
    Ok((host, target_entity_id))
}

fn line_inputs_json(inputs: &[LineInput]) -> String {
    let inputs = inputs
        .iter()
        .map(|input| {
            format!(
                "{{\"number\":{},\"channel_index\":{},\"gain_db\":{},\"phase\":{}}}",
                u32::from(input.channel_index) + 5,
                input.channel_index,
                input.gain_db,
                input.phase_inverted
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"inputs\":[{inputs}]}}")
}

fn headphone_outputs_json(outputs: &[HeadphoneOutput]) -> String {
    format!("{{\"outputs\":{}}}", headphone_outputs_array_json(outputs))
}

fn output_inventory_json(inventory: &OutputInventory) -> String {
    format!(
        "{{\"line_outputs\":{},\"headphone_outputs\":{}}}",
        line_outputs_array_json(&inventory.line_outputs),
        headphone_outputs_array_json(&inventory.headphone_outputs)
    )
}

fn line_outputs_array_json(outputs: &[LineOutput]) -> String {
    let outputs = outputs
        .iter()
        .map(|output| {
            let trim_db = output_trim_db_json(output.attenuation);
            let meter_path = meter_path_json(output.meter_path);
            format!(
                "{{\"number\":{},\"channel_index\":{},\"attenuation\":{},\"trim_db\":{trim_db},\"meter_path\":{meter_path}}}",
                u32::from(output.channel_index) + 1,
                output.channel_index,
                output.attenuation
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{outputs}]")
}

fn headphone_outputs_array_json(outputs: &[HeadphoneOutput]) -> String {
    let outputs = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let left_db = output_trim_db_json(output.attenuation[0]);
            let right_db = output_trim_db_json(output.attenuation[1]);
            let meter_path = meter_path_json(output.meter_path);
            format!(
                "{{\"number\":{},\"channel_indices\":[{},{}],\"attenuation\":[{},{}],\"trim_db\":[{left_db},{right_db}],\"meter_path\":{meter_path}}}",
                index + 1,
                output.channel_indices[0],
                output.channel_indices[1],
                output.attenuation[0],
                output.attenuation[1]
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{outputs}]")
}

fn meter_path_json(path: Option<MeterPath>) -> String {
    match path {
        Some(path) => format!(
            "{{\"property_id\":\"{:04x}\",\"record_index\":{},\"channel_index\":{}}}",
            path.property_id, path.record_index, path.channel_index
        ),
        None => "null".to_string(),
    }
}

fn output_trim_db_json(attenuation: u8) -> String {
    if attenuation == 100 {
        "null".to_string()
    } else {
        (-i16::from(attenuation)).to_string()
    }
}

fn mixer_meters_json(snapshot: &MixerMeters) -> String {
    let records = snapshot
        .records
        .iter()
        .map(mixer_meter_record_json)
        .collect::<Vec<_>>()
        .join(",");
    let faders = [
        ("main_host_11_12", MixerFader::MainHost11To12),
        ("headphone_host_11_12", MixerFader::HeadphoneHost11To12),
        ("main_line_in_5_6", MixerFader::MainLineIn5To6),
    ]
    .into_iter()
    .map(|(name, fader)| format!("\"{name}\":{}", mixer_fader_meter_json(snapshot, fader)))
    .collect::<Vec<_>>()
    .join(",");
    let age_ms = snapshot
        .updated_at
        .map(|updated_at| updated_at.elapsed().as_millis());
    format!(
        "{{\"status\":\"{}\",\"age_ms\":{},\"error\":{},\"faders\":{{{faders}}},\"records\":[{records}]}}",
        if snapshot.records.is_empty() { "starting" } else { "ok" },
        age_ms.map_or_else(|| "null".to_string(), |age| age.to_string()),
        snapshot
            .error
            .as_deref()
            .map_or_else(|| "null".to_string(), |error| format!("\"{}\"", json_escape(error)))
    )
}

fn mixer_meter_record_json(record: &MixerMeterRecord) -> String {
    let values = record
        .values
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let channels = record
        .channels()
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"property_id\":\"{:04x}\",\"index\":{},\"values\":[{values}],\"channels\":[{channels}]}}",
        record.property_id, record.index,
    )
}

fn mixer_fader_meter_json(snapshot: &MixerMeters, fader: MixerFader) -> String {
    let (property_id, index, slot) = fader.meter_slot();
    snapshot
        .records
        .iter()
        .find(|record| record.property_id == property_id && record.index == index)
        .and_then(|record| record.values.get(slot).copied())
        .map_or_else(|| "null".to_string(), |value| value.to_string())
}

fn datastore_entity_id(body: &str) -> Result<u64, String> {
    let marker = "\"uid\":\"";
    let start = body
        .find(marker)
        .map(|index| index + marker.len())
        .ok_or("datastore response did not include a uid")?;
    let value = body
        .get(start..start + 16)
        .ok_or("datastore uid is truncated")?;
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("datastore uid is invalid".to_string());
    }
    u64::from_str_radix(value, 16).map_err(|_| "datastore uid is invalid".to_string())
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
