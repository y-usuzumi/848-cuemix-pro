use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::avdecc::{probe as probe_avdecc, write_probe_result};
use crate::device::{datastore_write_request, DeviceClient, HttpResponse};
use crate::discovery::{discover_avdecc, write_discovery_results};
use crate::probe::{probe_device, write_probe_results};
use crate::server::serve;

pub(crate) fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };

    match command.as_str() {
        "discover" => {
            let options = CommonOptions::parse(args.collect())?;
            write_discovery_results(&discover_avdecc(options.timeout)?);
        }
        "avdecc-probe" => {
            let host = args.next().ok_or("missing AVDECC Proxy host")?;
            let options = AvdeccProbeOptions::parse(args.collect())?;
            write_probe_result(&probe_avdecc(
                &host,
                &options.path,
                options.request_entity_id.as_deref(),
                options.timeout,
            )?);
        }
        "probe" => {
            let host = args.next().ok_or("missing device host")?;
            let options = ReadOptions::parse(args.collect())?;
            let client = DeviceClient::new(&host, options.timeout)?;
            write_probe_results(&probe_device(&client), options.save)?;
        }
        "get" => {
            let host = args.next().ok_or("missing device host")?;
            let path = args.next().ok_or("missing path")?;
            let options = ReadOptions::parse(args.collect())?;
            let client = DeviceClient::new(&host, options.timeout)?;
            let response = client.request("GET", &path, None)?;
            print_response(&response);
            if let Some(path) = options.save {
                write_response_body(&response, path)?;
            }
        }
        "set" => {
            let host = args.next().ok_or("missing device host")?;
            let path = args.next().ok_or("missing path")?;
            let value = args.next().ok_or("missing value")?;
            let options = SetOptions::parse(args.collect())?;
            let client = DeviceClient::new(&host, options.timeout)?;
            let (request_path, body) = datastore_write_request(&path, &value)?;
            let response = client.request(&options.method, &request_path, Some(&body))?;
            print_response(&response);
            if response.status >= 400 {
                return Err(format!(
                    "device returned HTTP {} {}",
                    response.status, response.reason
                ));
            }
        }
        "serve" => {
            let host = args.next().ok_or("missing device host")?;
            let options = ServeOptions::parse(args.collect())?;
            serve(&host, &options.listen, options.timeout)?;
        }
        "help" | "--help" | "-h" => print_usage(),
        other => return Err(format!("unknown command '{other}'")),
    }

    Ok(())
}

fn print_usage() {
    eprintln!(
        "cuemix-848\n\n\
         Usage:\n\
           cuemix-848 discover [--timeout-ms n]\n\
           cuemix-848 avdecc-probe <host> [--path /] [--request-entity-id interface] [--timeout-ms n]\n\
           cuemix-848 probe <host> [--save file] [--timeout-ms n]\n\
           cuemix-848 get <host> <path> [--save file] [--timeout-ms n]\n\
           cuemix-848 set <host> <datastore-path> <value> [--method POST|PATCH] [--timeout-ms n]\n\
           cuemix-848 serve <host> [--listen 127.0.0.1:8480] [--timeout-ms n]\n\n\
         Host may be an IPv4 address, hostname, host:port, or http://host:port.\n\
         Use [ipv6-address] or [ipv6-address]:port for IPv6 hosts."
    );
}

#[derive(Clone, Copy)]
struct CommonOptions {
    timeout: Duration,
}

impl CommonOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut timeout = Duration::from_millis(2500);
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        Ok(Self { timeout })
    }
}

struct ReadOptions {
    timeout: Duration,
    save: Option<PathBuf>,
}

struct AvdeccProbeOptions {
    timeout: Duration,
    path: String,
    request_entity_id: Option<String>,
}

impl AvdeccProbeOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut timeout = Duration::from_millis(2500);
        let mut path = "/".to_string();
        let mut request_entity_id = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--path" => {
                    index += 1;
                    path = args
                        .get(index)
                        .ok_or("missing value for --path")?
                        .to_string();
                }
                "--request-entity-id" => {
                    index += 1;
                    request_entity_id = Some(
                        args.get(index)
                            .ok_or("missing interface for --request-entity-id")?
                            .to_string(),
                    );
                }
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        Ok(Self {
            timeout,
            path,
            request_entity_id,
        })
    }
}

impl ReadOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut timeout = Duration::from_millis(2500);
        let mut save = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--save" => {
                    index += 1;
                    save = Some(PathBuf::from(
                        args.get(index).ok_or("missing value for --save")?,
                    ));
                }
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        Ok(Self { timeout, save })
    }
}

struct SetOptions {
    timeout: Duration,
    method: String,
}

impl SetOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut timeout = Duration::from_millis(2500);
        let mut method = "POST".to_string();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--method" => {
                    index += 1;
                    method = args
                        .get(index)
                        .ok_or("missing value for --method")?
                        .to_uppercase();
                    if method != "POST" && method != "PATCH" {
                        return Err("--method must be POST or PATCH".to_string());
                    }
                }
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        Ok(Self { timeout, method })
    }
}

struct ServeOptions {
    listen: String,
    timeout: Duration,
}

impl ServeOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut listen = "127.0.0.1:8480".to_string();
        let mut timeout = Duration::from_millis(2500);
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--listen" => {
                    index += 1;
                    listen = args
                        .get(index)
                        .ok_or("missing value for --listen")?
                        .to_string();
                }
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        Ok(Self { listen, timeout })
    }
}

fn parse_timeout(value: Option<&String>) -> Result<Duration, String> {
    let milliseconds = value
        .ok_or("missing value for --timeout-ms")?
        .parse::<u64>()
        .map_err(|_| "invalid --timeout-ms value")?;
    Ok(Duration::from_millis(milliseconds))
}

fn write_response_body(response: &HttpResponse, path: PathBuf) -> Result<(), String> {
    std::fs::write(&path, &response.body).map_err(|err| format!("write {:?}: {err}", path))?;
    eprintln!("saved {}", path.display());
    Ok(())
}

fn print_response(response: &HttpResponse) {
    println!("HTTP {} {}", response.status, response.reason);
    for (name, value) in &response.headers {
        println!("{name}: {value}");
    }
    println!();
    print!("{}", response.body);
    if !response.body.ends_with('\n') {
        println!();
    }
}
