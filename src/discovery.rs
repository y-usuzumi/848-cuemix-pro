use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};
use std::process::Command;
use std::time::Duration;

use crate::device::json_escape;

#[derive(Clone)]
pub(crate) struct DiscoveryResult {
    instance: String,
    host: String,
    port: u16,
    addresses: Vec<String>,
    txt: Vec<String>,
}

#[derive(Default)]
struct AvahiDiscoveryBuilder {
    addresses: BTreeSet<String>,
    txt: BTreeSet<String>,
}

enum DnsRecordData {
    Domain(String),
    Srv { port: u16, target: String },
    Txt(Vec<String>),
    Address(IpAddr),
    Other,
}

struct DnsRecord {
    name: String,
    data: DnsRecordData,
}

pub(crate) fn discover_avdecc(timeout: Duration) -> Result<Vec<DiscoveryResult>, String> {
    let native_results = discover_avdecc_native(timeout)?;
    // Avahi can discover services advertised only on IPv6, which the native
    // stdlib-only probe cannot reliably multicast to without interface setup.
    let avahi_results = discover_avdecc_with_avahi()?;
    Ok(merge_discovery_results(native_results, avahi_results))
}

fn discover_avdecc_native(timeout: Duration) -> Result<Vec<DiscoveryResult>, String> {
    const SERVICE: &str = "_avdecc._tcp.local";
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind mDNS socket failed: {err}"))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|err| format!("set mDNS timeout failed: {err}"))?;
    socket
        .send_to(&mdns_query(SERVICE), "224.0.0.251:5353")
        .map_err(|err| format!("send mDNS query failed: {err}"))?;

    let mut records = Vec::new();
    let mut buffer = [0u8; 9000];
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((bytes, _)) => match parse_dns_records(&buffer[..bytes]) {
                Ok(packet_records) => records.extend(packet_records),
                Err(error) => eprintln!("ignoring malformed mDNS response: {error}"),
            },
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => return Err(format!("read mDNS response failed: {error}")),
        }
    }
    Ok(build_discovery_results(records, SERVICE))
}

fn discover_avdecc_with_avahi() -> Result<Vec<DiscoveryResult>, String> {
    let output = match Command::new("avahi-browse")
        .args(["--parsable", "--resolve", "--terminate", "_avdecc._tcp"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(_) | Err(_) => return Ok(Vec::new()),
    };
    Ok(parse_avahi_discovery(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_avahi_discovery(output: &str) -> Vec<DiscoveryResult> {
    let mut devices: BTreeMap<(String, String, u16), AvahiDiscoveryBuilder> = BTreeMap::new();

    for line in output.lines() {
        let parts = line.split(';').collect::<Vec<_>>();
        if parts.first() != Some(&"=") || parts.len() < 10 || parts[4] != "_avdecc._tcp" {
            continue;
        }
        let Ok(port) = parts[8].parse::<u16>() else {
            continue;
        };
        let instance = avahi_unescape(parts[3]);
        let host = avahi_unescape(parts[6]);
        let address = avahi_unescape(parts[7]);
        let txt = parse_avahi_txt(&parts[9..].join(";"));
        let entry = devices.entry((instance, host, port)).or_default();
        entry.addresses.insert(address);
        entry.txt.extend(txt);
    }

    devices
        .into_iter()
        .map(|((instance, host, port), builder)| DiscoveryResult {
            instance,
            host,
            port,
            addresses: builder.addresses.into_iter().collect(),
            txt: builder.txt.into_iter().collect(),
        })
        .collect()
}

fn merge_discovery_results(
    first: Vec<DiscoveryResult>,
    second: Vec<DiscoveryResult>,
) -> Vec<DiscoveryResult> {
    let mut merged: BTreeMap<(String, String, u16), AvahiDiscoveryBuilder> = BTreeMap::new();
    for result in first.into_iter().chain(second) {
        let entry = merged
            .entry((result.instance, result.host, result.port))
            .or_default();
        entry.addresses.extend(result.addresses);
        entry.txt.extend(result.txt);
    }
    merged
        .into_iter()
        .map(|((instance, host, port), builder)| DiscoveryResult {
            instance,
            host,
            port,
            addresses: builder.addresses.into_iter().collect(),
            txt: builder.txt.into_iter().collect(),
        })
        .collect()
}

fn avahi_unescape(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4].iter().all(u8::is_ascii_digit)
        {
            let digits = std::str::from_utf8(&bytes[index + 1..index + 4]).unwrap_or("");
            if let Ok(value) = digits.parse::<u8>() {
                out.push(value);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn parse_avahi_txt(input: &str) -> Vec<String> {
    let quoted = input
        .split('"')
        .enumerate()
        .filter(|(index, _)| *index % 2 == 1)
        .map(|(_, value)| avahi_unescape(value))
        .collect::<Vec<_>>();
    if quoted.is_empty() && !input.is_empty() {
        vec![avahi_unescape(input)]
    } else {
        quoted
    }
}

fn mdns_query(name: &str) -> Vec<u8> {
    let mut message = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    write_dns_name(&mut message, name);
    message.extend_from_slice(&12u16.to_be_bytes());
    message.extend_from_slice(&0x8001u16.to_be_bytes());
    message
}

fn parse_dns_records(bytes: &[u8]) -> Result<Vec<DnsRecord>, String> {
    if bytes.len() < 12 {
        return Err("mDNS response shorter than DNS header".to_string());
    }
    let question_count = read_u16(bytes, 4)? as usize;
    let answer_count = read_u16(bytes, 6)? as usize;
    let authority_count = read_u16(bytes, 8)? as usize;
    let additional_count = read_u16(bytes, 10)? as usize;
    let mut cursor = 12;

    for _ in 0..question_count {
        let (_, next) = read_dns_name(bytes, cursor)?;
        cursor = next
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or("truncated mDNS question")?;
    }

    let mut records = Vec::new();
    for _ in 0..answer_count + authority_count + additional_count {
        let (name, next) = read_dns_name(bytes, cursor)?;
        cursor = next;
        let record_type = read_u16(bytes, cursor)?;
        let rdata_len = read_u16(bytes, cursor + 8)? as usize;
        let rdata_start = cursor
            .checked_add(10)
            .ok_or("mDNS record offset overflow")?;
        let rdata_end = rdata_start
            .checked_add(rdata_len)
            .filter(|end| *end <= bytes.len())
            .ok_or("truncated mDNS record")?;
        let data = match record_type {
            1 if rdata_len == 4 => DnsRecordData::Address(IpAddr::V4(Ipv4Addr::new(
                bytes[rdata_start],
                bytes[rdata_start + 1],
                bytes[rdata_start + 2],
                bytes[rdata_start + 3],
            ))),
            12 => DnsRecordData::Domain(read_dns_name(bytes, rdata_start)?.0),
            16 => DnsRecordData::Txt(parse_dns_txt(&bytes[rdata_start..rdata_end])),
            28 if rdata_len == 16 => {
                let octets: [u8; 16] = bytes[rdata_start..rdata_end]
                    .try_into()
                    .map_err(|_| "invalid mDNS AAAA record")?;
                DnsRecordData::Address(IpAddr::V6(Ipv6Addr::from(octets)))
            }
            33 if rdata_len >= 6 => DnsRecordData::Srv {
                port: read_u16(bytes, rdata_start + 4)?,
                target: read_dns_name(bytes, rdata_start + 6)?.0,
            },
            _ => DnsRecordData::Other,
        };
        records.push(DnsRecord { name, data });
        cursor = rdata_end;
    }
    Ok(records)
}

fn read_u16(bytes: &[u8], index: usize) -> Result<u16, String> {
    let end = index.checked_add(2).ok_or("mDNS offset overflow")?;
    let pair: [u8; 2] = bytes
        .get(index..end)
        .ok_or("truncated mDNS integer")?
        .try_into()
        .map_err(|_| "invalid mDNS integer")?;
    Ok(u16::from_be_bytes(pair))
}

fn read_dns_name(bytes: &[u8], start: usize) -> Result<(String, usize), String> {
    let mut labels = Vec::new();
    let mut cursor = start;
    let mut return_cursor = None;
    for _ in 0..128 {
        let length = *bytes.get(cursor).ok_or("truncated mDNS name")?;
        if length & 0xc0 == 0xc0 {
            let next = *bytes.get(cursor + 1).ok_or("truncated mDNS pointer")?;
            let offset = (((length & 0x3f) as usize) << 8) | next as usize;
            if return_cursor.is_none() {
                return_cursor = Some(cursor + 2);
            }
            cursor = offset;
            continue;
        }
        if length & 0xc0 != 0 {
            return Err("invalid mDNS label type".to_string());
        }
        if length == 0 {
            return Ok((labels.join("."), return_cursor.unwrap_or(cursor + 1)));
        }
        let label_start = cursor + 1;
        let label_end = label_start
            .checked_add(length as usize)
            .filter(|end| *end <= bytes.len())
            .ok_or("truncated mDNS label")?;
        labels.push(String::from_utf8_lossy(&bytes[label_start..label_end]).to_string());
        cursor = label_end;
    }
    Err("mDNS name exceeded pointer limit".to_string())
}

fn write_dns_name(out: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

fn parse_dns_txt(bytes: &[u8]) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let length = bytes[cursor] as usize;
        let start = cursor + 1;
        let end = start + length;
        if end > bytes.len() {
            break;
        }
        values.push(String::from_utf8_lossy(&bytes[start..end]).to_string());
        cursor = end;
    }
    values
}

fn build_discovery_results(records: Vec<DnsRecord>, service: &str) -> Vec<DiscoveryResult> {
    let service = service.to_ascii_lowercase();
    let mut instances = BTreeSet::new();
    let mut srv_records = BTreeMap::new();
    let mut txt_records = BTreeMap::new();
    let mut addresses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for record in records {
        let name = record.name.to_ascii_lowercase();
        match record.data {
            DnsRecordData::Domain(target) if name == service => {
                instances.insert(target.to_ascii_lowercase());
            }
            DnsRecordData::Srv { port, target } if name.ends_with(&service) => {
                instances.insert(name.clone());
                srv_records.insert(name, (port, target.to_ascii_lowercase()));
            }
            DnsRecordData::Txt(values) if name.ends_with(&service) => {
                instances.insert(name.clone());
                txt_records.insert(name, values);
            }
            DnsRecordData::Address(address) => {
                addresses
                    .entry(name)
                    .or_default()
                    .insert(address.to_string());
            }
            _ => {}
        }
    }

    instances
        .into_iter()
        .filter_map(|instance| {
            let (port, host) = srv_records.get(&instance)?.clone();
            let txt = txt_records.remove(&instance).unwrap_or_default();
            Some(DiscoveryResult {
                instance,
                addresses: addresses
                    .get(&host)
                    .map(|values| values.iter().cloned().collect())
                    .unwrap_or_default(),
                host,
                port,
                txt,
            })
        })
        .collect()
}

pub(crate) fn write_discovery_results(results: &[DiscoveryResult]) {
    for result in results {
        let addresses = result
            .addresses
            .iter()
            .map(|address| format!("\"{}\"", json_escape(address)))
            .collect::<Vec<_>>()
            .join(",");
        let txt = result
            .txt
            .iter()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{{\"instance\":\"{}\",\"host\":\"{}\",\"port\":{},\"addresses\":[{}],\"txt\":[{}]}}",
            json_escape(&result.instance),
            json_escape(&result.host),
            result.port,
            addresses,
            txt
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_standard_avdecc_mdns_query() {
        let query = mdns_query("_avdecc._tcp.local");
        assert_eq!(&query[..12], &[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            read_dns_name(&query, 12),
            Ok(("_avdecc._tcp.local".to_string(), query.len() - 4))
        );
        assert_eq!(&query[query.len() - 4..], &[0, 12, 128, 1]);
    }

    #[test]
    fn assembles_and_merges_avdecc_discovery_records() {
        let instance = "848._avdecc._tcp.local".to_string();
        let host = "848afeb9e2.local".to_string();
        let results = build_discovery_results(
            vec![
                DnsRecord {
                    name: "_avdecc._tcp.local".to_string(),
                    data: DnsRecordData::Domain(instance.clone()),
                },
                DnsRecord {
                    name: instance.clone(),
                    data: DnsRecordData::Srv {
                        port: 17221,
                        target: host.clone(),
                    },
                },
                DnsRecord {
                    name: instance,
                    data: DnsRecordData::Txt(vec!["Manufacturer=MOTU".to_string()]),
                },
                DnsRecord {
                    name: host,
                    data: DnsRecordData::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 4, 166))),
                },
            ],
            "_avdecc._tcp.local",
        );
        let merged = merge_discovery_results(
            results,
            vec![DiscoveryResult {
                instance: "848._avdecc._tcp.local".to_string(),
                host: "848afeb9e2.local".to_string(),
                port: 17221,
                addresses: vec!["fe80::1".to_string()],
                txt: vec!["Version=1".to_string()],
            }],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].port, 17221);
        assert_eq!(merged[0].addresses, vec!["192.168.4.166", "fe80::1"]);
        assert_eq!(merged[0].txt, vec!["Manufacturer=MOTU", "Version=1"]);
    }

    #[test]
    fn parses_avahi_avdecc_record() {
        let output = r#"=;eth2;IPv4;848\032\040848AFEB9E2\041;_avdecc._tcp;local;848AFEB9E2.local;192.168.4.166;17221;"Version=1" "Manufacturer=MOTU" "com.motu.type=proaudiov2""#;
        let results = parse_avahi_discovery(output);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].instance, "848 (848AFEB9E2)");
        assert_eq!(results[0].host, "848AFEB9E2.local");
        assert_eq!(results[0].addresses, vec!["192.168.4.166"]);
        assert_eq!(
            results[0].txt,
            vec!["Manufacturer=MOTU", "Version=1", "com.motu.type=proaudiov2"]
        );
    }
}
