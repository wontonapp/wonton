use std::net::Ipv4Addr;

use ipnet::Ipv4Net;

// /proc/net/route 的地址按宿主字节序打印；默认路由不是已占用地址段，否则整个池都会被挡住。
pub(super) fn parse(text: &str) -> Result<Vec<Ipv4Net>, String> {
    let mut subnets = Vec::new();
    for line in text.lines().skip(1).filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let bad = || format!("cannot parse a line of the host routing table: {line}");
        fields.next().ok_or_else(bad)?;
        let address = fields.next().ok_or_else(bad)?;
        fields.next().ok_or_else(bad)?;
        let flags = fields.next().ok_or_else(bad)?;
        let mask = fields.nth(3).ok_or_else(bad)?;
        let number = |field| u32::from_str_radix(field, 16).map_err(|_| bad());
        let flags = number(flags)?;
        if flags & 1 == 0 {
            continue;
        }
        let mask = u32::from_be_bytes(number(mask)?.to_ne_bytes());
        if mask == 0 {
            continue;
        }
        let prefix = mask.leading_ones() as u8;
        if prefix == 0 || mask != u32::MAX << (32 - prefix) {
            return Err(bad());
        }
        let address = Ipv4Addr::from(number(address)?.to_ne_bytes());
        let subnet = Ipv4Net::new(address, prefix).map_err(|_| bad())?.trunc();
        subnets.push(subnet);
    }
    Ok(subnets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_include_vpn_subnets_and_hosts_but_not_defaults_or_down_routes() {
        let text = "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\n\
            eth0 00000000 0100A8C0 0003 0 0 100 00000000 0 0 0\n\
            vpn0 0000D50A 00000000 0001 0 0 0 00FFFFFF 0 0 0\n\
            vpn0 0101D50A 00000000 0005 0 0 0 FFFFFFFF 0 0 0\n\
            down0 0000D40A 00000000 0000 0 0 0 0000FFFF 0 0 0\n";
        assert_eq!(
            parse(text).unwrap(),
            [
                "10.213.0.0/24".parse::<Ipv4Net>().unwrap(),
                "10.213.1.1/32".parse().unwrap()
            ]
        );
        assert!(parse("header\nbroken").is_err());
    }
}
