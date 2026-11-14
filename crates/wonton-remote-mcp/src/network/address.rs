use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::config::{AddressPolicy, Config};

use super::NetworkError;

pub(super) fn validate_url(url: &Url, policy: AddressPolicy) -> Result<&str, NetworkError> {
    if (url.scheme() != "https"
        && !(policy == AddressPolicy::AllowPrivate && url.scheme() == "http"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(NetworkError::InvalidInput);
    }
    let host = url.host_str().ok_or(NetworkError::InvalidInput)?;
    if policy == AddressPolicy::PublicOnly {
        let Host::Domain(domain) = url.host().ok_or(NetworkError::InvalidInput)? else {
            return Err(NetworkError::BlockedAddress);
        };
        let domain = domain.trim_end_matches('.');
        if !domain.contains('.')
            || domain == "localhost"
            || domain.ends_with(".localhost")
            || domain.split('.').any(str::is_empty)
        {
            return Err(NetworkError::BlockedAddress);
        }
    }
    Ok(host)
}

pub(super) fn validate_addresses(
    addresses: &[IpAddr],
    config: &Config,
) -> Result<(), NetworkError> {
    if addresses.is_empty() {
        return Err(NetworkError::Unreachable);
    }
    for address in addresses {
        let normalized = match address {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(*address),
            IpAddr::V4(_) => *address,
        };
        if config
            .blocked_networks
            .iter()
            .any(|network| network.contains(address) || network.contains(&normalized))
            || (config.address_policy == AddressPolicy::PublicOnly && !is_public(normalized))
        {
            return Err(NetworkError::BlockedAddress);
        }
    }
    Ok(())
}

fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_ipv4(address),
        IpAddr::V6(address) => public_ipv6(address),
    }
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !matches!(
        (a, b, c),
        (0 | 10 | 127 | 224..=255, _, _)
            | (100, 64..=127, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0 | 2)
            | (192, 88, 99)
            | (192, 168, _)
            | (198, 18 | 19, _)
            | (198, 51, 100)
            | (203, 0, 113)
    )
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    let [first, second, ..] = address.segments();
    // Permit allocated global unicast only. Special-purpose transition ranges
    // are excluded too: they can route an embedded IPv4 address outside this check.
    (first & 0xe000) == 0x2000
        && !(first == 0x2001 && (second < 0x0200 || second == 0x0db8))
        && first != 0x2002
        && !(first == 0x3fff && (second & 0xf000) == 0)
}
