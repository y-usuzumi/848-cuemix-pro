use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::avdecc::{probe as probe_avdecc, write_probe_result, DescriptorRead};
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
                options.read_entity_descriptor,
                options.read_configuration_descriptor,
                options.read_descriptor,
                ProbeTiming {
                    timeout: options.timeout,
                    listen: options.listen,
                },
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
            let (host, options) = parse_serve_command(args.collect())?;
            serve(host.as_deref(), &options.listen, options.timeout)?;
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
           cuemix-848 avdecc-probe <host> [--path /] [--request-entity-id interface] [--read-entity-descriptor id|--read-configuration-descriptor id|--read-descriptor id type index] [--timeout-ms n]\n\
           cuemix-848 probe <host> [--save file] [--timeout-ms n]\n\
           cuemix-848 get <host> <path> [--save file] [--timeout-ms n]\n\
           cuemix-848 set <host> <datastore-path> <value> [--method POST|PATCH] [--timeout-ms n]\n\
           cuemix-848 serve <host> [--listen 127.0.0.1:8480] [--timeout-ms n]\n\n\
         Host may be an IPv4 address, hostname, host:port, or http://host:port.\n\
         Use [ipv6-address] or [ipv6-address]:port for IPv6 hosts; link-local IPv6 may include a scope, for example [fe80::1%eth2]."
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
    listen: Duration,
    path: String,
    request_entity_id: Option<String>,
    read_entity_descriptor: Option<u64>,
    read_configuration_descriptor: Option<u64>,
    read_descriptor: Option<DescriptorRead>,
}

impl AvdeccProbeOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut timeout = Duration::from_millis(2500);
        let mut listen = Duration::from_millis(250);
        let mut path = "/".to_string();
        let mut request_entity_id = None;
        let mut read_entity_descriptor = None;
        let mut read_configuration_descriptor = None;
        let mut read_descriptor = None;
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
                "--listen-ms" => {
                    index += 1;
                    listen = parse_listen_duration(args.get(index))?;
                }
                "--request-entity-id" => {
                    index += 1;
                    request_entity_id = Some(
                        args.get(index)
                            .ok_or("missing interface for --request-entity-id")?
                            .to_string(),
                    );
                }
                "--read-entity-descriptor" => {
                    index += 1;
                    read_entity_descriptor = Some(parse_entity_id(
                        args.get(index)
                            .ok_or("missing entity ID for --read-entity-descriptor")?,
                    )?);
                }
                "--read-configuration-descriptor" => {
                    index += 1;
                    read_configuration_descriptor = Some(parse_entity_id(
                        args.get(index)
                            .ok_or("missing entity ID for --read-configuration-descriptor")?,
                    )?);
                }
                "--read-descriptor" => {
                    index += 1;
                    let target_entity_id = parse_entity_id(
                        args.get(index)
                            .ok_or("missing entity ID for --read-descriptor")?,
                    )?;
                    index += 1;
                    let descriptor_type = parse_u16(
                        args.get(index)
                            .ok_or("missing descriptor type for --read-descriptor")?,
                        "descriptor type",
                    )?;
                    index += 1;
                    let descriptor_index = parse_u16(
                        args.get(index)
                            .ok_or("missing descriptor index for --read-descriptor")?,
                        "descriptor index",
                    )?;
                    read_descriptor = Some(DescriptorRead {
                        target_entity_id,
                        descriptor_type,
                        descriptor_index,
                    });
                }
                "--timeout-ms" => {
                    index += 1;
                    timeout = parse_timeout(args.get(index))?;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
            index += 1;
        }
        let descriptor_requests = read_entity_descriptor.is_some() as u8
            + read_configuration_descriptor.is_some() as u8
            + read_descriptor.is_some() as u8;
        if descriptor_requests > 1 {
            return Err("request only one descriptor per AVDECC probe".to_string());
        }
        Ok(Self {
            timeout,
            listen,
            path,
            request_entity_id,
            read_entity_descriptor,
            read_configuration_descriptor,
            read_descriptor,
        })
    }
}

fn parse_entity_id(value: &str) -> Result<u64, String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("entity ID must be exactly 16 hexadecimal digits".to_string());
    }
    u64::from_str_radix(value, 16).map_err(|_| "invalid entity ID".to_string())
}

fn parse_u16(value: &str, name: &str) -> Result<u16, String> {
    let (radix, value) = value
        .strip_prefix("0x")
        .map(|value| (16, value))
        .unwrap_or((10, value));
    u16::from_str_radix(value, radix).map_err(|_| format!("invalid {name}"))
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

fn parse_serve_command(mut args: Vec<String>) -> Result<(Option<String>, ServeOptions), String> {
    let host = args
        .first()
        .filter(|value| !value.starts_with('-'))
        .cloned();
    if host.is_some() {
        args.remove(0);
    }
    Ok((host, ServeOptions::parse(args)?))
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

fn parse_listen_duration(value: Option<&String>) -> Result<Duration, String> {
    let milliseconds = value
        .ok_or("missing value for --listen-ms")?
        .parse::<u64>()
        .map_err(|_| "invalid --listen-ms value")?;
    let duration = Duration::from_millis(milliseconds);
    if duration.is_zero() || duration > Duration::from_secs(30) {
        return Err("--listen-ms must be between 1 and 30000".to_string());
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_generic_read_descriptor_request() {
        let options = AvdeccProbeOptions::parse(vec![
            "--request-entity-id".to_string(),
            "eth2".to_string(),
            "--read-descriptor".to_string(),
            "0001f2fffefeb9e2".to_string(),
            "0x001a".to_string(),
            "0".to_string(),
        ])
        .expect("valid options");
        let request = options.read_descriptor.expect("descriptor request");
        assert_eq!(request.target_entity_id, 0x0001_f2ff_fefe_b9e2);
        assert_eq!(request.descriptor_type, 0x001a);
        assert_eq!(request.descriptor_index, 0);
    }

    #[test]
    fn rejects_multiple_descriptor_requests() {
        assert!(AvdeccProbeOptions::parse(vec![
            "--read-entity-descriptor".to_string(),
            "0001f2fffefeb9e2".to_string(),
            "--read-configuration-descriptor".to_string(),
            "0001f2fffefeb9e2".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn parses_hostless_and_fixed_host_server_commands() {
        let (host, options) =
            parse_serve_command(vec!["--listen".to_string(), "127.0.0.1:0".to_string()])
                .expect("hostless server command");
        assert_eq!(host, None);
        assert_eq!(options.listen, "127.0.0.1:0");

        let (host, _) = parse_serve_command(vec!["192.168.4.166".to_string()])
            .expect("fixed host server command");
        assert_eq!(host.as_deref(), Some("192.168.4.166"));
    }

    #[test]
    fn parses_a_bounded_passive_listen_duration() {
        let options =
            AvdeccProbeOptions::parse(vec!["--listen-ms".to_string(), "15000".to_string()])
                .expect("valid options");
        assert_eq!(options.listen, Duration::from_secs(15));
        assert!(
            AvdeccProbeOptions::parse(vec!["--listen-ms".to_string(), "0".to_string()]).is_err()
        );
        assert!(
            AvdeccProbeOptions::parse(vec!["--listen-ms".to_string(), "30001".to_string(),])
                .is_err()
        );
    }
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
use crate::avdecc::ProbeTiming;
