//! The server address: exactly `http://HOST:PORT` with a loopback host, no TLS (W1-43 Q-8, EN-05).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The default server, the port the server and the installer publish (W1-43 Q-8).
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8601";

/// A validated address. `localhost` is stored as `127.0.0.1`, so no name is ever resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    ip: IpAddr,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error(
        "the server address must start with http:// (TLS is not in this build; unix:// is not in 0.2.001)"
    )]
    Scheme,
    #[error(
        "the server host must be a loopback address (127.0.0.0/8, ::1 or localhost): a token is never sent in clear text to another host"
    )]
    NotLoopback,
    #[error(
        "the server address must be exactly http://HOST:PORT (no user, path, query or fragment)"
    )]
    Shape,
    #[error("the server port must be a number from 1 to 65535")]
    Port,
}

impl Endpoint {
    /// Exactly `http://HOST:PORT`: HOST is `localhost`, an address in 127.0.0.0/8 or `[::1]`; PORT is 1 to 65535
    /// without a leading zero. Nothing is resolved: `localhost` becomes 127.0.0.1.
    pub fn parse(text: &str) -> Result<Endpoint, EndpointError> {
        let rest = text.strip_prefix("http://").ok_or(EndpointError::Scheme)?;
        if rest
            .bytes()
            .any(|b| matches!(b, b'@' | b'/' | b'?' | b'#') || b <= 0x20 || b >= 0x7f)
        {
            return Err(EndpointError::Shape);
        }
        let (host, port_text) = if let Some(inner) = rest.strip_prefix('[') {
            let close = inner.find(']').ok_or(EndpointError::Shape)?;
            let port_text = inner[close + 1..]
                .strip_prefix(':')
                .ok_or(EndpointError::Shape)?;
            (&inner[..close], port_text)
        } else {
            rest.rsplit_once(':').ok_or(EndpointError::Shape)?
        };
        let bracketed = rest.starts_with('[');

        let ip = if bracketed {
            let v6: Ipv6Addr = host.parse().map_err(|_| EndpointError::Shape)?;
            if !v6.is_loopback() {
                return Err(EndpointError::NotLoopback);
            }
            IpAddr::V6(v6)
        } else if host.eq_ignore_ascii_case("localhost") {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            match host.parse::<Ipv4Addr>() {
                Ok(v4) if v4.is_loopback() => IpAddr::V4(v4),
                _ => return Err(EndpointError::NotLoopback),
            }
        };

        let canonical = !port_text.is_empty()
            && port_text.len() <= 5
            && port_text.bytes().all(|b| b.is_ascii_digit())
            && !port_text.starts_with('0');
        let port: u16 = if canonical {
            port_text.parse().map_err(|_| EndpointError::Port)?
        } else {
            0
        };
        if port == 0 {
            return Err(EndpointError::Port);
        }
        Ok(Endpoint { ip, port })
    }

    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ip {
            IpAddr::V4(a) => write!(f, "http://{a}:{}", self.port),
            IpAddr::V6(a) => write!(f, "http://[{a}]:{}", self.port),
        }
    }
}
