use std::net::{Ipv6Addr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub(super) fn read_interface_mac(interface: &str) -> Result<[u8; 6], String> {
    if interface.is_empty()
        || !interface.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        return Err("interface name contains unsupported characters".to_string());
    }
    let value = std::fs::read_to_string(format!("/sys/class/net/{interface}/address"))
        .map_err(|error| format!("read MAC address for {interface} failed: {error}"))?;
    parse_mac_address(value.trim())
}

pub(super) fn parse_mac_address(value: &str) -> Result<[u8; 6], String> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 6 {
        return Err("MAC address must contain six octets".to_string());
    }
    let mut address = [0; 6];
    for (index, part) in parts.iter().enumerate() {
        address[index] = u8::from_str_radix(part, 16).map_err(|_| "invalid MAC address")?;
    }
    Ok(address)
}

pub(super) struct ProxyAddress {
    pub(super) host_header: String,
    pub(super) socket_address: String,
}

pub(super) fn parse_proxy_address(host: &str) -> Result<ProxyAddress, String> {
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
        validate_scoped_ipv6(address).map_err(|_| "invalid bracketed IPv6 AVDECC Proxy host")?;
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
            host_header: format!("{}:{port}", http_ipv6_host(address)),
            socket_address: format!("[{address}]:{port}"),
        });
    }
    if validate_scoped_ipv6(host).is_ok() {
        return Ok(ProxyAddress {
            host_header: format!("{}:17221", http_ipv6_host(host)),
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

fn validate_scoped_ipv6(address: &str) -> Result<(), ()> {
    let (address, scope) = match address.split_once('%') {
        Some((address, scope)) => (address, Some(scope)),
        None => (address, None),
    };
    address.parse::<Ipv6Addr>().map_err(|_| ())?;
    if let Some(scope) = scope {
        if scope.is_empty()
            || scope.contains('%')
            || !scope.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
        {
            return Err(());
        }
    }
    Ok(())
}

fn http_ipv6_host(address: &str) -> String {
    format!("[{}]", address.replace('%', "%25"))
}

pub(super) fn validate_proxy_path(path: &str) -> Result<(), String> {
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

pub(super) fn connect_with_timeout(address: &str, timeout: Duration) -> Result<TcpStream, String> {
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
