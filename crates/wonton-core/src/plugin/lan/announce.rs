//! 启动器声明的宣告地址（`WONTON_ANNOUNCE_ADDRESSES`）：手机要连的地址，不是虚拟机里的地址。核心跑在桌面虚拟机里时，
//! 虚拟机自己的网卡地址手机连不到，启动器在宿主上知道真实地址，交进来。
//!
//! 每一项可以带前缀（`192.168.1.20/24`），出网的直连网段用它（ADR-0023）；宣告时只用地址。不带前缀时 IPv4 按 /24、
//! IPv6 按 /64。

use std::net::IpAddr;

use ipnet::IpNet;

use super::is_private;

/// 一个宣告地址和它的前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Announced {
    pub ip: IpAddr,
    pub prefix: Option<u8>,
}

impl Announced {
    /// 出网的直连网段：带前缀的用它，不带时 IPv4 按 /24、IPv6 按 /64。
    pub fn cidr(&self) -> Option<IpNet> {
        let prefix = self
            .prefix
            .unwrap_or(if self.ip.is_ipv4() { 24 } else { 64 });
        IpNet::new(self.ip, prefix).ok().map(|net| net.trunc())
    }
}

/// 核心容器的 `WONTON_ANNOUNCE_ADDRESSES`：逗号分隔的私有地址，可以带 `/前缀`，同一个地址只算一次。
pub fn parse(text: &str) -> Result<Vec<Announced>, String> {
    let mut addresses: Vec<Announced> = Vec::new();
    for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (ip, prefix) = match item.split_once('/') {
            Some((ip, prefix)) => (
                ip,
                Some(
                    prefix
                        .parse::<u8>()
                        .map_err(|_| format!("{item}: the prefix length is not a number"))?,
                ),
            ),
            None => (item, None),
        };
        let ip: IpAddr = ip
            .parse()
            .map_err(|_| format!("{item} is not an IP address"))?;
        if !is_private(ip) {
            return Err(format!(
                "{ip} is not a private address: only 10/8, 172.16/12, 192.168/16, 100.64/10 and fc00::/7 are allowed"
            ));
        }
        let bits = if ip.is_ipv4() { 32 } else { 128 };
        if let Some(prefix) = prefix
            && prefix > bits
        {
            return Err(format!("{item}: the prefix length is longer than {bits}"));
        }
        match addresses.iter_mut().find(|a| a.ip == ip) {
            Some(existing) => existing.prefix = existing.prefix.or(prefix),
            None => addresses.push(Announced { ip, prefix }),
        }
    }
    if addresses.is_empty() {
        return Err("no address given".to_owned());
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announced_addresses_must_be_private_and_keep_their_prefix() {
        let parsed = parse(" 192.168.1.20/24, fd00::10/64, 192.168.1.20 ").unwrap();
        assert_eq!(parsed.len(), 2, "同一个地址只算一次");
        assert_eq!(parsed[0].ip.to_string(), "192.168.1.20");
        assert_eq!(parsed[0].prefix, Some(24));
        assert_eq!(parsed[1].prefix, Some(64));
        // 不带前缀按 /24、/64；重复的后面带了前缀时用上前缀。
        let nets: Vec<String> = parse("192.168.1.20, fd00::10")
            .unwrap()
            .iter()
            .map(|a| a.cidr().unwrap().to_string())
            .collect();
        assert_eq!(nets, ["192.168.1.0/24", "fd00::/64"]);
        let kept = parse("192.168.1.20, 192.168.1.20/16").unwrap();
        assert_eq!(kept[0].prefix, Some(16));
        assert!(parse("192.168.1.20,203.0.113.5").is_err());
        assert!(parse("192.168.1.20/33").is_err());
        assert!(parse("192.168.1.20/24/1").is_err());
        assert!(parse(" , ").is_err());
    }
}
