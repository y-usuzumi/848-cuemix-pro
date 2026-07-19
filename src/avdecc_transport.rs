use std::net::{Ipv6Addr, TcpStream, ToSocketAddrs};
use std::time::Duration;

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
