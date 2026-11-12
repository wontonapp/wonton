//! 插件的局域网端口开在哪里（ADR-0019 的“接口：App 和核心”）。
//!
//! 核心的管理接口 `/_wonton/api/lan` 收发这里的类型，客户端核心原样交给壳。地址都是 IP 地址的文本，不带端口和
//! 前缀长度。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other` 或 `Unknown`，不让整份设置解析失败。

use serde::{Deserialize, Serialize};

/// `GET /_wonton/api/lan` 和 `PUT /_wonton/api/lan` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanSettings {
    /// 主人在设置里的选择；没选过时是 [`LanChoice::Auto`]。
    pub choice: LanChoice,
    /// 绑定地址由核心容器的 `WONTON_LAN_ADDRESSES` 或 `WONTON_ANNOUNCE_ADDRESSES` 定死：`choice` 不起作用，设置里不能改。
    pub fixed_by_environment: bool,
    /// 上面的环境变量里是 `WONTON_ANNOUNCE_ADDRESSES`：端口在虚拟机里全接口绑定，手机连的是宣告地址。
    pub announce_by_environment: bool,
    /// 端口开在这些地址上；选不出地址时为空，原因见 `problem`。由启动器声明宣告地址时是通配地址 `0.0.0.0`（有 IPv6 时还有 `::`），
    /// 手机连的是 `announce_addresses`。
    pub addresses: Vec<LanAddress>,
    /// 告诉手机去连的地址。没设 `WONTON_ANNOUNCE_ADDRESSES` 时和 `addresses` 相同；设了时是它。
    pub announce_addresses: Vec<LanAddress>,
    /// 选不出地址，或者选的网卡、地址不在宿主上了；没有问题时为空。
    pub problem: Option<LanProblem>,
    /// 可以选的网卡：宿主上有私有地址的网卡，不列 Docker 的网桥和 veth。默认路由所在的网卡在前。
    pub interfaces: Vec<LanInterface>,
    /// 开了局域网端口的插件，按安装的先后。
    pub plugins: Vec<LanPlugin>,
}

/// 端口开在哪些地址上：主人的选择，也是 `PUT /_wonton/api/lan` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LanChoice {
    /// 核心自动选：宿主默认路由所在网卡的私有 IPv4 主地址。默认路由走的是虚拟网卡（Tailscale、WireGuard 等）时选不出。
    Auto,
    /// 主人选的网卡和地址，合在一起用，至少一项。选网卡时用它上面的私有 IPv4 地址，DHCP 换了地址也跟着换；
    /// IPv6 地址只能逐个选。
    Manual {
        /// 网卡名，如 `eno1`、`tailscale0`。
        interfaces: Vec<String>,
        /// 私有地址，如 `192.168.0.193`、`fd00::10`。
        addresses: Vec<String>,
    },
    /// 比这个 App 新的核心给的选法，不能用来改设置。
    #[serde(other)]
    Unknown,
}

/// 宿主上的一个私有地址。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanAddress {
    /// 如 `192.168.0.193`。
    pub address: String,
    /// 地址所在的网卡；`WONTON_LAN_ADDRESSES` 指定的地址在宿主上找不到时为空。
    pub interface: Option<String>,
    /// 这个地址上的连接经 docker-proxy 转发，插件看到的客户端地址是网桥网关，不是真实的设备。插件网络只有 IPv4，
    /// IPv6 地址都是这样。插件按客户端地址做的限制和日志在这个地址上都不准，设置里要写明。
    pub proxied: bool,
}

/// 宿主上的一块网卡。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanInterface {
    /// 如 `eno1`。
    pub name: String,
    pub kind: LanInterfaceKind,
    /// 网卡上的私有地址，IPv4 在前；公网、环回和链路本地地址不列。
    pub addresses: Vec<LanAddress>,
    /// 宿主的默认路由走这块网卡，自动选看的就是它。
    pub default_route: bool,
}

/// 网卡的类型。隧道和 WireGuard 不会被自动选。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LanInterfaceKind {
    Ethernet,
    Wifi,
    /// 主人自己建的网桥，如虚拟机主机上的 `br0`。
    Bridge,
    /// tun 设备，如 Tailscale。
    Tunnel,
    #[serde(rename = "wireguard")]
    WireGuard,
    /// VLAN、bond 等，以及比这个 App 新的类型。
    #[serde(other)]
    Other,
}

/// 端口开不出去的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanProblem {
    pub reason: LanFailure,
    /// 核心写的说明，带上网卡名和地址，如“默认路由走的是 tailscale0（虚拟网卡）”。
    pub message: String,
}

/// 选不出地址的原因，壳据此提示主人去选网卡，或者等家里的服务器修好。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LanFailure {
    /// 宿主没有默认路由。
    NoDefaultRoute,
    /// 默认路由走的是虚拟网卡（如 Tailscale 的出口节点、WireGuard），不自动选。
    VirtualInterface,
    /// 服务器跑在电脑上的虚拟机里（Docker Desktop、OrbStack、WSL2 这类），虚拟机自己的地址手机连不到，也没有声明宣告地址。
    DesktopVm,
    /// 网卡上没有私有 IPv4 地址：自动选的那块，或者主人选的网卡。
    NoPrivateAddress,
    /// 主人选的网卡或地址，或者 `WONTON_LAN_ADDRESSES` 里的地址，不在宿主上了。
    Missing,
    /// 检测用的容器起不来，或者读不懂它的输出。
    DetectionFailed,
    #[serde(other)]
    Other,
}

/// 一个开了局域网端口的插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanPlugin {
    /// 插件 ID。
    pub id: String,
    pub title: String,
    /// 清单 `lan.ports` 里的端口，宿主端口和容器端口相同。
    pub ports: Vec<LanPort>,
    /// 端口没开出去的原因（如按新地址重建容器失败），同插件在已安装列表里异常的原因；开着时为空。
    pub problem: Option<String>,
}

/// 一个局域网端口。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanPort {
    pub port: u16,
    pub protocol: LanProtocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LanProtocol {
    Tcp,
    Udp,
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    fn address(address: &str, interface: &str, proxied: bool) -> LanAddress {
        LanAddress {
            address: address.into(),
            interface: Some(interface.into()),
            proxied,
        }
    }

    fn settings() -> LanSettings {
        LanSettings {
            choice: LanChoice::Manual {
                interfaces: vec!["eno1".into()],
                addresses: vec!["fd00::10".into()],
            },
            fixed_by_environment: false,
            announce_by_environment: false,
            addresses: vec![
                address("192.168.0.193", "eno1", false),
                address("fd00::10", "eno1", true),
            ],
            announce_addresses: vec![
                address("192.168.0.193", "eno1", false),
                address("fd00::10", "eno1", true),
            ],
            problem: None,
            interfaces: vec![
                LanInterface {
                    name: "eno1".into(),
                    kind: LanInterfaceKind::Ethernet,
                    addresses: vec![
                        address("192.168.0.193", "eno1", false),
                        address("fd00::10", "eno1", true),
                    ],
                    default_route: true,
                },
                LanInterface {
                    name: "tailscale0".into(),
                    kind: LanInterfaceKind::Tunnel,
                    addresses: vec![address("100.101.102.103", "tailscale0", false)],
                    default_route: false,
                },
            ],
            plugins: vec![LanPlugin {
                id: "app.samba".into(),
                title: "Samba".into(),
                ports: vec![LanPort {
                    port: 445,
                    protocol: LanProtocol::Tcp,
                }],
                problem: Some("按新地址重建容器失败：port is already allocated".into()),
            }],
        }
    }

    /// 契约：管理接口的类型都能经 JSON 往返。
    #[test]
    fn lan_types_round_trip_through_json() {
        assert_eq!(round_trip(&settings()), settings());
        let failed = LanSettings {
            choice: LanChoice::Auto,
            fixed_by_environment: true,
            announce_by_environment: false,
            addresses: vec![],
            announce_addresses: vec![],
            problem: Some(LanProblem {
                reason: LanFailure::VirtualInterface,
                message: "默认路由走的是 tailscale0（虚拟网卡）".into(),
            }),
            interfaces: vec![],
            plugins: vec![],
        };
        assert_eq!(round_trip(&failed), failed);
        // 启动器声明了宣告地址：绑定是通配地址，手机连的是宣告地址。
        let announced = LanSettings {
            choice: LanChoice::Auto,
            fixed_by_environment: true,
            announce_by_environment: true,
            addresses: vec![address("0.0.0.0", "", false)],
            announce_addresses: vec![address("192.168.0.5", "", true)],
            problem: None,
            interfaces: vec![],
            plugins: vec![],
        };
        assert_eq!(round_trip(&announced), announced);
    }

    /// 契约：核心和客户端核心之间的 JSON 写法。改了就要两边一起改，旧的 App 也会读不懂。
    #[test]
    fn lan_wire_format_is_stable() {
        assert_eq!(
            serde_json::to_value(LanChoice::Auto).unwrap(),
            json!({"mode": "auto"})
        );
        let choice: LanChoice = serde_json::from_value(json!({
            "mode": "manual",
            "interfaces": ["eno1"],
            "addresses": ["fd00::10"],
        }))
        .unwrap();
        assert_eq!(
            choice,
            LanChoice::Manual {
                interfaces: vec!["eno1".into()],
                addresses: vec!["fd00::10".into()],
            }
        );
        assert_eq!(
            serde_json::to_value(LanInterfaceKind::WireGuard).unwrap(),
            json!("wireguard")
        );
        assert_eq!(
            serde_json::to_value(LanFailure::NoPrivateAddress).unwrap(),
            json!("no_private_address")
        );
        assert_eq!(
            serde_json::to_value(LanFailure::DesktopVm).unwrap(),
            json!("desktop_vm")
        );
        assert_eq!(
            serde_json::to_value(LanPort {
                port: 22000,
                protocol: LanProtocol::Udp,
            })
            .unwrap(),
            json!({"port": 22000, "protocol": "udp"})
        );
    }

    /// 核心可能比 App 新：不认识的取值落到兜底的变体，多出来的字段被忽略，整份设置照样能读。
    #[test]
    fn lan_values_from_a_newer_core_still_parse() {
        let mut value = serde_json::to_value(settings()).unwrap();
        value["choice"] = json!({"mode": "per_plugin", "plugins": {}});
        value["interfaces"][0]["kind"] = json!("infiniband");
        value["plugins"][0]["ports"][0]["protocol"] = json!("sctp");
        value["problem"] = json!({"reason": "address_conflict", "message": "x"});
        value["added_in_a_later_version"] = json!(true);
        let parsed: LanSettings = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.choice, LanChoice::Unknown);
        assert_eq!(parsed.interfaces[0].kind, LanInterfaceKind::Other);
        assert_eq!(parsed.plugins[0].ports[0].protocol, LanProtocol::Other);
        assert_eq!(parsed.problem.unwrap().reason, LanFailure::Other);
        assert!(!parsed.announce_by_environment);
        assert_eq!(parsed.announce_addresses.len(), 2);
        let desktop: LanFailure = serde_json::from_value(json!("desktop_vm")).unwrap();
        assert_eq!(desktop, LanFailure::DesktopVm);
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
