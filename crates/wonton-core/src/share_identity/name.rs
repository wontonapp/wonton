use std::net::IpAddr;

use ruma_common::ServerName;
use url::Host;
use wonton_protocol::ShareIdentityFailure;

/// Identity comparison always uses a normalized host and the Matrix default port (8448).
/// `explicit_port` is deliberately separate: discovery must distinguish `host` from `host:8448`.
#[derive(Clone, Debug)]
pub struct MatrixServerName {
    pub(super) host: String,
    pub(super) explicit_port: Option<u16>,
}

impl MatrixServerName {
    pub fn parse(input: &str) -> Result<Self, ShareIdentityFailure> {
        let fail = ShareIdentityFailure::DiscoveryFailed;
        if input.is_empty() || input.len() > 1024 || input.chars().any(char::is_whitespace) {
            return Err(fail);
        }
        let end = if input.starts_with('[') {
            input.find(']').map(|i| i + 1).ok_or(fail)?
        } else {
            input.find(':').unwrap_or(input.len())
        };
        let raw_host = &input[..end];
        // Host::parse supplies IDNA and IPv6 canonicalization, but URL metacharacters and
        // numeric shorthand must not turn an invalid Matrix server name into a valid URL.
        if raw_host.contains(['/', '\\', '@', '?', '#', '%']) {
            return Err(fail);
        }
        let parsed_host = Host::parse(raw_host).map_err(|_| fail)?;
        if let Host::Ipv4(ip) = &parsed_host
            && raw_host.parse::<std::net::Ipv4Addr>().ok().as_ref() != Some(ip)
        {
            return Err(fail);
        }
        let host = parsed_host.to_string();
        let explicit_port = if input.len() == end {
            None
        } else {
            let port = input[end..].strip_prefix(':').ok_or(fail)?;
            if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(fail);
            }
            Some(port.parse::<u16>().ok().filter(|p| *p != 0).ok_or(fail)?)
        };
        let name = Self {
            host,
            explicit_port,
        };
        let authority = name.authority();
        <&ServerName>::try_from(authority.as_str()).map_err(|_| fail)?;
        if let Host::Domain(domain) = Host::parse(&name.host).map_err(|_| fail)?
            && (domain.len() > 253
                || domain.split('.').any(|label| {
                    label.is_empty()
                        || label.len() > 63
                        || label.starts_with('-')
                        || label.ends_with('-')
                }))
        {
            return Err(fail);
        }
        Ok(name)
    }

    pub fn authority(&self) -> String {
        match self.explicit_port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.host == other.host
            && self.explicit_port.unwrap_or(8448) == other.explicit_port.unwrap_or(8448)
    }

    pub(super) fn ip(&self) -> Option<IpAddr> {
        self.host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .ok()
    }
}
