//! 读宿主的路由和网卡（`wonton-core lan-probe`）。核心在容器里时，这段代码跑在宿主网络里的一次性小容器里（见
//! [`super::detect`]），所以只读、只打印：从 `/proc/net/route` 读 main 表的默认路由，经 netlink 列出地址，从
//! `/sys/class/net` 认出网卡的类型。选哪个地址在核心里算（[`super::select`]）。
//!
//! `/proc/net/route` 只列 main 表：Tailscale 用出口节点时，策略路由把流量导到 tailscale0，`ip route get` 查到的就是
//! 它，main 表里的默认路由还是物理网卡（ADR-0019 的“别的检测办法”）。

use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

/// 子命令名：`wonton-core lan-probe`。
pub const COMMAND: &str = "lan-probe";

/// 一次性容器的用户：nobody。
pub const NOBODY_USER: &str = "65534:65534";

/// 宿主的网络：检测容器打印的 JSON。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostNetwork {
    /// main 表里的 IPv4 默认路由。
    pub routes: Vec<DefaultRoute>,
    pub interfaces: Vec<HostInterface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultRoute {
    pub interface: String,
    pub metric: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInterface {
    pub name: String,
    /// `uevent` 里的 `DEVTYPE`：`bridge`、`wlan`、`wireguard`、`vlan` 等；以太网卡、veth 和 tun 没有。
    pub devtype: Option<String>,
    /// tun/tap 设备（有 `tun_flags`），如 Tailscale。
    pub tun: bool,
    /// 有 `device`：背后是真实的硬件。
    pub physical: bool,
    /// 环回网卡（`type` 是 772）。
    pub loopback: bool,
    /// `iflink` 和 `ifindex` 不同：veth 这类成对的、或者叠在别的网卡上的设备。
    pub linked: bool,
    /// 按内核列出的顺序：同一网段的主地址在从地址前面。
    pub addresses: Vec<HostAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAddress {
    pub ip: IpAddr,
    /// IPv4 的从地址：同一网段里后加上去的地址。
    pub secondary: bool,
    /// 网段的前缀长度：出口容器放行宿主所在网段时用（ADR-0023）。
    pub prefix: u8,
}

/// 子命令的入口：结果以 JSON 打印在标准输出上，出错时原因写在标准错误上、以 1 退出。
pub fn command() -> ExitCode {
    match probe(Path::new("/")) {
        Ok(network) => match serde_json::to_string(&network) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("输出检测结果失败：{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// 读 `root` 下的 `/proc` 和 `/sys`，再经 netlink 列出本网络命名空间里的地址。
pub fn probe(root: &Path) -> Result<HostNetwork, String> {
    let route = root.join("proc/net/route");
    let text =
        fs::read_to_string(&route).map_err(|e| format!("读 {} 失败：{e}", route.display()))?;
    assemble(&text, &root.join("sys/class/net"), addresses()?)
}

/// 把三处读到的拼起来：地址按网卡序号归到网卡上。
pub fn assemble(
    route: &str,
    sys_net: &Path,
    addresses: Vec<(u32, HostAddress)>,
) -> Result<HostNetwork, String> {
    let mut interfaces = interfaces(sys_net)?;
    for (index, address) in addresses {
        if let Some((_, interface)) = interfaces.iter_mut().find(|(i, _)| *i == index) {
            interface.addresses.push(address);
        }
    }
    Ok(HostNetwork {
        routes: default_routes(route),
        interfaces: interfaces.into_iter().map(|(_, i)| i).collect(),
    })
}

/// `/proc/net/route` 里的默认路由：目的和掩码都是 0、已启用（`RTF_UP`）的行。
pub fn default_routes(text: &str) -> Vec<DefaultRoute> {
    const RTF_UP: u32 = 0x1;
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (name, destination, flags, metric, mask) = match fields.as_slice() {
                [name, destination, _, flags, _, _, metric, mask, ..] => {
                    (*name, *destination, *flags, *metric, *mask)
                }
                _ => return None,
            };
            let up = u32::from_str_radix(flags, 16).is_ok_and(|f| f & RTF_UP != 0);
            (destination == "00000000" && mask == "00000000" && up).then(|| DefaultRoute {
                interface: name.to_owned(),
                metric: metric.parse().unwrap_or(u32::MAX),
            })
        })
        .collect()
}

/// `/sys/class/net` 下的网卡和它们的序号，按名字排序。
fn interfaces(dir: &Path) -> Result<Vec<(u32, HostInterface)>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("读 {} 失败：{e}", dir.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("读 {} 失败：{e}", dir.display()))?;
        let path = entry.path();
        // bonding_masters 这类文件不是网卡。
        if !path.is_dir() {
            continue;
        }
        let number = |file: &str| {
            fs::read_to_string(path.join(file))
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok())
        };
        let Some(index) = number("ifindex") else {
            continue;
        };
        let uevent = fs::read_to_string(path.join("uevent")).unwrap_or_default();
        let interface = HostInterface {
            name: entry.file_name().to_string_lossy().into_owned(),
            devtype: uevent
                .lines()
                .find_map(|line| line.strip_prefix("DEVTYPE="))
                .map(str::to_owned),
            tun: path.join("tun_flags").exists(),
            physical: path.join("device").exists(),
            loopback: number("type") == Some(772),
            linked: number("iflink").is_some_and(|link| link != index),
            addresses: Vec::new(),
        };
        found.push((index, interface));
    }
    found.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(found)
}

/// 经 netlink（`RTM_GETADDR`）列出所有地址：（网卡序号，地址）。
#[cfg(target_os = "linux")]
fn addresses() -> Result<Vec<(u32, HostAddress)>, String> {
    use netlink_packet_core::{
        NLM_F_DUMP, NLM_F_REQUEST, NetlinkHeader, NetlinkMessage, NetlinkPayload,
    };
    use netlink_packet_route::RouteNetlinkMessage;
    use netlink_packet_route::address::AddressMessage;
    use netlink_sys::protocols::NETLINK_ROUTE;
    use netlink_sys::{Socket, SocketAddr};

    let failed = |what: &str, e: &dyn std::fmt::Display| format!("经 netlink {what}失败：{e}");
    let mut socket = Socket::new(NETLINK_ROUTE).map_err(|e| failed("打开套接字", &e))?;
    socket.bind_auto().map_err(|e| failed("绑定", &e))?;
    socket
        .connect(&SocketAddr::new(0, 0))
        .map_err(|e| failed("连接内核", &e))?;
    let mut header = NetlinkHeader::default();
    header.flags = NLM_F_REQUEST | NLM_F_DUMP;
    let payload = RouteNetlinkMessage::GetAddress(AddressMessage::default());
    let mut request = NetlinkMessage::new(header, NetlinkPayload::from(payload));
    request.finalize();
    let mut buf = vec![0; request.buffer_len()];
    request.serialize(&mut buf);
    socket.send(&buf, 0).map_err(|e| failed("请求地址", &e))?;

    let mut found = Vec::new();
    let mut received = vec![0; 1 << 16];
    loop {
        let size = socket
            .recv(&mut &mut received[..], 0)
            .map_err(|e| failed("读地址", &e))?;
        let mut offset = 0;
        while offset < size {
            let message =
                NetlinkMessage::<RouteNetlinkMessage>::deserialize(&received[offset..size])
                    .map_err(|e| failed("解析地址", &e))?;
            match message.payload {
                NetlinkPayload::Done(_) => return Ok(found),
                NetlinkPayload::Error(e) => return Err(failed("列地址", &format!("{e:?}"))),
                NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address)) => {
                    found.extend(host_address(&address));
                }
                _ => {}
            }
            let length = message.header.length as usize;
            if length == 0 {
                break;
            }
            // 消息按 4 字节对齐。
            offset += (length + 3) & !3;
        }
    }
}

#[cfg(target_os = "linux")]
fn host_address(
    message: &netlink_packet_route::address::AddressMessage,
) -> Option<(u32, HostAddress)> {
    use netlink_packet_route::address::{AddressAttribute, AddressHeaderFlags};
    // 点对点的网卡（WireGuard、tun）上 IFA_ADDRESS 是对端，本机的地址在 IFA_LOCAL。
    let local = message.attributes.iter().find_map(|a| match a {
        AddressAttribute::Local(ip) => Some(*ip),
        _ => None,
    });
    let address = message.attributes.iter().find_map(|a| match a {
        AddressAttribute::Address(ip) => Some(*ip),
        _ => None,
    });
    let ip = local.or(address)?;
    let secondary = message.header.flags.contains(AddressHeaderFlags::Secondary);
    let prefix = message.header.prefix_len;
    let address = HostAddress {
        ip,
        secondary,
        prefix,
    };
    Some((message.header.index, address))
}

/// 宿主服务端只支持 Linux（ADR-0002）；在别的系统上编译时如实报错，不猜。
#[cfg(not(target_os = "linux"))]
fn addresses() -> Result<Vec<(u32, HostAddress)>, String> {
    Err("只能在 Linux 上检测网卡的地址".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// npchy（2026-11-02）的 `/proc/net/route`：有线和 Wi-Fi 各一条默认路由，另有 Docker 网桥和 Tailscale 的网段。
    const ROUTE: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eno1\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
wlp2s0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0
docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eno1\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";

    #[test]
    fn default_routes_come_from_the_main_table_with_their_metrics() {
        assert_eq!(
            default_routes(ROUTE),
            [
                DefaultRoute {
                    interface: "eno1".into(),
                    metric: 100
                },
                DefaultRoute {
                    interface: "wlp2s0".into(),
                    metric: 600
                },
            ]
        );
        // 没启用的默认路由不算。
        let down = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                    eth0\t00000000\t0100A8C0\t0002\t0\t0\t0\t00000000\n";
        assert!(default_routes(down).is_empty());
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn interface_kinds_come_from_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        let net = dir.path();
        let nic = |name: &str, index: u32, link: u32, uevent: &str| {
            write(&net.join(name).join("ifindex"), &format!("{index}\n"));
            write(&net.join(name).join("iflink"), &format!("{link}\n"));
            write(&net.join(name).join("type"), "1\n");
            write(&net.join(name).join("uevent"), uevent);
        };
        nic("eno1", 2, 2, "INTERFACE=eno1\nIFINDEX=2\n");
        fs::create_dir_all(net.join("eno1/device")).unwrap();
        nic("wlp2s0", 3, 3, "DEVTYPE=wlan\nINTERFACE=wlp2s0\n");
        nic("tailscale0", 4, 4, "INTERFACE=tailscale0\n");
        write(&net.join("tailscale0/tun_flags"), "0x1002\n");
        nic("veth1a2b", 9, 8, "INTERFACE=veth1a2b\n");
        nic("lo", 1, 1, "INTERFACE=lo\n");
        write(&net.join("lo/type"), "772\n");
        write(&net.join("bonding_masters"), "");
        let addresses = vec![
            (
                2,
                HostAddress {
                    ip: "192.168.0.193".parse().unwrap(),
                    secondary: false,
                    prefix: 24,
                },
            ),
            (
                42,
                HostAddress {
                    ip: "10.9.9.9".parse().unwrap(),
                    secondary: false,
                    prefix: 24,
                },
            ),
        ];
        let network = assemble(ROUTE, net, addresses).unwrap();
        let names: Vec<&str> = network.interfaces.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["eno1", "lo", "tailscale0", "veth1a2b", "wlp2s0"]);
        let by_name = |name: &str| network.interfaces.iter().find(|i| i.name == name).unwrap();
        assert!(by_name("eno1").physical && by_name("eno1").devtype.is_none());
        assert_eq!(
            by_name("eno1").addresses.len(),
            1,
            "序号对不上的地址不归到任何网卡"
        );
        assert_eq!(by_name("wlp2s0").devtype.as_deref(), Some("wlan"));
        assert!(by_name("tailscale0").tun);
        assert!(by_name("veth1a2b").linked);
        assert!(by_name("lo").loopback);
    }
}
