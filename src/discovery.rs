use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use crate::device::json_escape;

#[derive(Clone)]
pub(crate) struct DiscoveryResult {
    instance: String,
    host: String,
    port: u16,
    addresses: Vec<String>,
    txt: Vec<String>,
}

#[cfg(test)]
#[derive(Default)]
struct DiscoveryBuilder {
    addresses: BTreeSet<String>,
    txt: BTreeSet<String>,
}

enum DnsRecordData {
    Domain(String),
    Srv {
        port: u16,
        target: String,
    },
    Txt(Vec<String>),
    Address {
        address: IpAddr,
        scope: Option<String>,
    },
    Other,
}

struct DnsRecord {
    name: String,
    data: DnsRecordData,
}

struct DiscoverySocket {
    socket: UdpSocket,
    scope: Option<String>,
}

struct MulticastInterface {
    name: String,
    index: u32,
}

pub(crate) fn discover_avdecc(timeout: Duration) -> Result<Vec<DiscoveryResult>, String> {
    discover_avdecc_native(timeout)
}

fn discover_avdecc_native(timeout: Duration) -> Result<Vec<DiscoveryResult>, String> {
    const SERVICE: &str = "_avdecc._tcp.local";
    let query = mdns_query(SERVICE);
    let mut sockets = open_ipv4_mdns_sockets(&query)?;
    for interface in ipv6_multicast_interfaces() {
        let socket = match UdpSocket::bind("[::]:0") {
            Ok(socket) => socket,
            Err(error) => {
                eprintln!(
                    "ignoring IPv6 mDNS interface {}: bind failed: {error}",
                    interface.name
                );
                continue;
            }
        };
        if let Err(error) = socket.set_nonblocking(true) {
            eprintln!(
                "ignoring IPv6 mDNS interface {}: set nonblocking mode failed: {error}",
                interface.name
            );
            continue;
        }
        let destination = SocketAddrV6::new(
            Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x00fb),
            5353,
            0,
            interface.index,
        );
        if let Err(error) = socket.send_to(&query, destination) {
            eprintln!(
                "ignoring IPv6 mDNS interface {}: send query failed: {error}",
                interface.name
            );
            continue;
        }
        sockets.push(DiscoverySocket {
            socket,
            scope: Some(interface.name),
        });
    }

    let mut records = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut buffer = [0u8; 9000];
    loop {
        let mut received = false;
        for socket in &sockets {
            loop {
                match socket.socket.recv_from(&mut buffer) {
                    Ok((bytes, _)) => {
                        received = true;
                        match parse_dns_records_scoped(&buffer[..bytes], socket.scope.as_deref()) {
                            Ok(packet_records) => records.extend(packet_records),
                            Err(error) => eprintln!("ignoring malformed mDNS response: {error}"),
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        eprintln!("ignoring mDNS receive error: {error}");
                        break;
                    }
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining == Duration::ZERO {
            break;
        }
        if !received {
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
    Ok(build_discovery_results(records, SERVICE))
}

fn open_ipv4_mdns_sockets(query: &[u8]) -> Result<Vec<DiscoverySocket>, String> {
    let addresses = ipv4_multicast_addresses();
    let addresses = if addresses.is_empty() {
        vec![Ipv4Addr::UNSPECIFIED]
    } else {
        addresses
    };
    let mut sockets = Vec::new();
    let mut last_error = None;
    for address in addresses {
        let socket = match UdpSocket::bind(SocketAddrV4::new(address, 0)) {
            Ok(socket) => socket,
            Err(error) => {
                last_error = Some(format!("bind source {address} failed: {error}"));
                continue;
            }
        };
        if let Err(error) = socket.set_nonblocking(true) {
            last_error = Some(format!(
                "set source {address} nonblocking mode failed: {error}"
            ));
            continue;
        }
        if let Err(error) = socket.send_to(query, "224.0.0.251:5353") {
            last_error = Some(format!("send from source {address} failed: {error}"));
            continue;
        }
        sockets.push(DiscoverySocket {
            socket,
            scope: None,
        });
    }
    if sockets.is_empty() {
        return Err(format!(
            "send IPv4 mDNS query failed: {}",
            last_error.unwrap_or_else(|| "no eligible IPv4 interface".to_string())
        ));
    }
    Ok(sockets)
}

fn ipv4_multicast_addresses() -> Vec<Ipv4Addr> {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        platform_ipv4::addresses()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

fn unique_ipv4_addresses(mut addresses: Vec<Ipv4Addr>) -> Vec<Ipv4Addr> {
    addresses.retain(|address| !address.is_unspecified() && !address.is_loopback());
    addresses.sort_unstable();
    addresses.dedup();
    addresses
}

#[cfg(target_os = "linux")]
mod platform_ipv4 {
    use super::{unique_ipv4_addresses, Ipv4Addr};
    use std::os::raw::{c_char, c_int};

    const AF_INET: u16 = 2;
    const IFF_UP: u32 = 0x1;
    const IFF_LOOPBACK: u32 = 0x8;
    const IFF_MULTICAST: u32 = 0x1000;

    #[repr(C)]
    struct SockAddr {
        family: u16,
        data: [u8; 14],
    }

    #[repr(C)]
    struct SockAddrIn {
        family: u16,
        port: u16,
        address: [u8; 4],
        zero: [u8; 8],
    }

    #[repr(C)]
    struct IfAddrs {
        next: *mut IfAddrs,
        name: *mut c_char,
        flags: u32,
        address: *mut SockAddr,
        netmask: *mut SockAddr,
        destination: *mut SockAddr,
        data: *mut core::ffi::c_void,
    }

    extern "C" {
        fn getifaddrs(addresses: *mut *mut IfAddrs) -> c_int;
        fn freeifaddrs(addresses: *mut IfAddrs);
    }

    pub(super) fn addresses() -> Vec<Ipv4Addr> {
        let mut list = core::ptr::null_mut();
        if unsafe { getifaddrs(&mut list) } != 0 {
            return Vec::new();
        }
        let mut addresses = Vec::new();
        let mut current = list;
        while !current.is_null() {
            let interface = unsafe { &*current };
            if interface.flags & (IFF_UP | IFF_MULTICAST) == IFF_UP | IFF_MULTICAST
                && interface.flags & IFF_LOOPBACK == 0
                && !interface.address.is_null()
            {
                let socket = unsafe { &*interface.address };
                if socket.family == AF_INET {
                    let socket = unsafe { &*(interface.address as *const SockAddrIn) };
                    addresses.push(Ipv4Addr::from(socket.address));
                }
            }
            current = interface.next;
        }
        unsafe { freeifaddrs(list) };
        unique_ipv4_addresses(addresses)
    }
}

#[cfg(target_os = "windows")]
mod platform_ipv4 {
    use super::{unique_ipv4_addresses, Ipv4Addr};
    use core::ffi::c_void;
    use std::mem::size_of;
    use std::os::raw::c_char;

    const AF_INET: u16 = 2;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const ERROR_SUCCESS: u32 = 0;
    const IF_OPER_STATUS_UP: u32 = 1;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    const IP_ADAPTER_NO_MULTICAST: u32 = 0x10;

    #[repr(C)]
    struct SockAddr {
        family: u16,
        data: [u8; 14],
    }

    #[repr(C)]
    struct SockAddrIn {
        family: u16,
        port: u16,
        address: [u8; 4],
        zero: [u8; 8],
    }

    #[repr(C)]
    struct SocketAddress {
        address: *const SockAddr,
        length: i32,
    }

    #[repr(C)]
    struct AdapterUnicastAddress {
        length: u32,
        flags: u32,
        next: *mut AdapterUnicastAddress,
        address: SocketAddress,
    }

    #[repr(C)]
    struct AdapterAddresses {
        length: u32,
        interface_index: u32,
        next: *mut AdapterAddresses,
        adapter_name: *mut c_char,
        first_unicast_address: *mut AdapterUnicastAddress,
        first_anycast_address: *mut c_void,
        first_multicast_address: *mut c_void,
        first_dns_server_address: *mut c_void,
        dns_suffix: *mut u16,
        description: *mut u16,
        friendly_name: *mut u16,
        physical_address: [u8; 8],
        physical_address_length: u32,
        flags: u32,
        mtu: u32,
        interface_type: u32,
        oper_status: u32,
    }

    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetAdaptersAddresses(
            family: u32,
            flags: u32,
            reserved: *mut c_void,
            addresses: *mut AdapterAddresses,
            size: *mut u32,
        ) -> u32;
    }

    pub(super) fn addresses() -> Vec<Ipv4Addr> {
        let mut requested_size = 15 * 1024usize;
        for _ in 0..2 {
            let words = requested_size
                .checked_add(size_of::<usize>() - 1)
                .map(|size| size / size_of::<usize>())
                .unwrap_or(0);
            if words == 0 {
                return Vec::new();
            }
            let mut buffer = vec![0usize; words];
            let mut buffer_size = (buffer.len() * size_of::<usize>()) as u32;
            let status = unsafe {
                GetAdaptersAddresses(
                    u32::from(AF_INET),
                    0,
                    core::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut buffer_size,
                )
            };
            if status == ERROR_BUFFER_OVERFLOW {
                requested_size = (buffer_size as usize).saturating_add(1024);
                continue;
            }
            if status != ERROR_SUCCESS {
                return Vec::new();
            }
            let mut addresses = Vec::new();
            let mut adapter = buffer.as_ptr().cast::<AdapterAddresses>();
            while !adapter.is_null() {
                let current = unsafe { &*adapter };
                if current.oper_status == IF_OPER_STATUS_UP
                    && current.interface_type != IF_TYPE_SOFTWARE_LOOPBACK
                    && current.flags & IP_ADAPTER_NO_MULTICAST == 0
                {
                    let mut unicast = current.first_unicast_address;
                    while !unicast.is_null() {
                        let address = unsafe { &*unicast };
                        if address.address.length as usize >= size_of::<SockAddrIn>()
                            && !address.address.address.is_null()
                        {
                            let socket = unsafe { &*address.address.address };
                            if socket.family == AF_INET {
                                let socket =
                                    unsafe { &*(address.address.address as *const SockAddrIn) };
                                addresses.push(Ipv4Addr::from(socket.address));
                            }
                        }
                        unicast = address.next;
                    }
                }
                adapter = current.next;
            }
            return unique_ipv4_addresses(addresses);
        }
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn ipv6_multicast_interfaces() -> Vec<MulticastInterface> {
    const IFF_UP: u32 = 0x1;
    const IFF_LOOPBACK: u32 = 0x8;
    const IFF_MULTICAST: u32 = 0x1000;

    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("native IPv6 mDNS interface enumeration unavailable: {error}");
            return Vec::new();
        }
    };
    let mut interfaces = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
        {
            continue;
        }
        let flags = std::fs::read_to_string(format!("/sys/class/net/{name}/flags"))
            .ok()
            .and_then(|value| parse_interface_flags(&value));
        let Some(flags) = flags else {
            continue;
        };
        if flags & (IFF_UP | IFF_MULTICAST) != IFF_UP | IFF_MULTICAST || flags & IFF_LOOPBACK != 0 {
            continue;
        }
        let index = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|index| *index != 0);
        if let Some(index) = index {
            interfaces.push(MulticastInterface { name, index });
        }
    }
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    interfaces
}

#[cfg(not(target_os = "linux"))]
fn ipv6_multicast_interfaces() -> Vec<MulticastInterface> {
    Vec::new()
}

#[cfg(any(target_os = "linux", test))]
fn parse_interface_flags(value: &str) -> Option<u32> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u32::from_str_radix(value, 16).ok()
}

fn mdns_query(name: &str) -> Vec<u8> {
    let mut message = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    write_dns_name(&mut message, name);
    message.extend_from_slice(&12u16.to_be_bytes());
    message.extend_from_slice(&0x8001u16.to_be_bytes());
    message
}

fn parse_dns_records_scoped(
    bytes: &[u8],
    interface_scope: Option<&str>,
) -> Result<Vec<DnsRecord>, String> {
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
            1 if rdata_len == 4 => DnsRecordData::Address {
                address: IpAddr::V4(Ipv4Addr::new(
                    bytes[rdata_start],
                    bytes[rdata_start + 1],
                    bytes[rdata_start + 2],
                    bytes[rdata_start + 3],
                )),
                scope: None,
            },
            12 => DnsRecordData::Domain(read_dns_name(bytes, rdata_start)?.0),
            16 => DnsRecordData::Txt(parse_dns_txt(&bytes[rdata_start..rdata_end])),
            28 if rdata_len == 16 => {
                let octets: [u8; 16] = bytes[rdata_start..rdata_end]
                    .try_into()
                    .map_err(|_| "invalid mDNS AAAA record")?;
                DnsRecordData::Address {
                    address: IpAddr::V6(Ipv6Addr::from(octets)),
                    scope: interface_scope.map(str::to_string),
                }
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
            DnsRecordData::Address { address, scope } => {
                addresses
                    .entry(name)
                    .or_default()
                    .insert(discovery_address(address, scope.as_deref()));
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

fn discovery_address(address: IpAddr, scope: Option<&str>) -> String {
    match (address, scope) {
        (IpAddr::V6(address), Some(scope)) if address.is_unicast_link_local() => {
            format!("{address}%{scope}")
        }
        (address, _) => address.to_string(),
    }
}

#[cfg(test)]
fn merge_discovery_results(
    first: Vec<DiscoveryResult>,
    second: Vec<DiscoveryResult>,
) -> Vec<DiscoveryResult> {
    let mut merged: BTreeMap<(String, String, u16), DiscoveryBuilder> = BTreeMap::new();
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
                    data: DnsRecordData::Address {
                        address: IpAddr::V4(Ipv4Addr::new(192, 168, 4, 166)),
                        scope: None,
                    },
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
    fn retains_scope_for_link_local_ipv6_addresses() {
        let instance = "848._avdecc._tcp.local".to_string();
        let host = "848afeb9e2.local".to_string();
        let results = build_discovery_results(
            vec![
                DnsRecord {
                    name: "_avdecc._tcp.local".to_string(),
                    data: DnsRecordData::Domain(instance.clone()),
                },
                DnsRecord {
                    name: instance,
                    data: DnsRecordData::Srv {
                        port: 17221,
                        target: host.clone(),
                    },
                },
                DnsRecord {
                    name: host,
                    data: DnsRecordData::Address {
                        address: IpAddr::V6(Ipv6Addr::LOCALHOST),
                        scope: Some("eth2".to_string()),
                    },
                },
                DnsRecord {
                    name: "848afeb9e2.local".to_string(),
                    data: DnsRecordData::Address {
                        address: IpAddr::V6("fe80::1".parse().unwrap()),
                        scope: Some("eth2".to_string()),
                    },
                },
            ],
            "_avdecc._tcp.local",
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].addresses, vec!["::1", "fe80::1%eth2"]);
    }

    #[test]
    fn deduplicates_eligible_ipv4_sources() {
        assert_eq!(
            unique_ipv4_addresses(vec![
                Ipv4Addr::new(192, 168, 4, 2),
                Ipv4Addr::new(127, 0, 0, 1),
                Ipv4Addr::UNSPECIFIED,
                Ipv4Addr::new(192, 168, 4, 2),
                Ipv4Addr::new(10, 0, 0, 2),
            ]),
            vec![Ipv4Addr::new(10, 0, 0, 2), Ipv4Addr::new(192, 168, 4, 2)]
        );
    }

    #[test]
    fn parses_linux_interface_flags() {
        assert_eq!(parse_interface_flags("0x1003\n"), Some(0x1003));
        assert_eq!(parse_interface_flags("1003"), Some(0x1003));
        assert_eq!(parse_interface_flags("not-a-flag"), None);
    }
}
