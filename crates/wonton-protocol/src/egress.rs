//! 出网插件走宿主的代理（ADR-0023 的“接口：App 和核心”）。
//!
//! 核心的管理接口 `/_wonton/api/egress-proxy` 收发这里的类型，客户端核心原样交给壳。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other` 或 `Unknown`，不让整份设置解析失败。

use serde::{Deserialize, Serialize};

use crate::PluginProblem;

/// `GET /_wonton/api/egress-proxy` 和 `PUT /_wonton/api/egress-proxy` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProxySettings {
    /// 主人的代理地址，密码打码；没配代理或者用的是代理插件时为空。
    pub url: Option<String>,
    pub source: EgressProxySource,
    /// 装着的代理插件（清单声明了 `provides.egress_proxy`，ADR-0039）；没装时为空。装着不等于在用，在用时
    /// `source` 是 [`EgressProxySource::Plugin`]。
    pub provider: Option<EgressProxyProvider>,
    /// 插件的路径：经出口容器到主人的代理，再到外网。代理连不上时挡安装，见 [`EgressFailure`]。还没探测过时为空。
    pub plugin_path: Option<EgressProbe>,
    /// 核心自己的路径：核心直接连主人的代理。结果只影响 GitHub 和官方索引，不挡安装。还没探测过时为空。
    pub core_path: Option<EgressProbe>,
    /// 宿主所在网段：出口容器对它们直连，不经主人的代理。
    pub host_cidrs: Vec<String>,
    /// 主人额外填的直连网段（比如 Tailscale 的 `100.64.0.0/10`）。这些网段也由出口容器直连，出网插件都能连。
    pub direct_cidrs: Vec<String>,
    /// 核心写的说明，壳原样显示：放不了行的网段（和 Docker 网络重叠、查不到前缀）。没有时为空。
    pub notices: Vec<String>,
    /// 第一次配上或者彻底取消代理时要重启的插件：要主人先确认，确认的列表放在 [`EgressProxyUpdate::restarts`] 带回。
    pub restarts: Vec<EgressRestart>,
}

/// 代理地址从哪里来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum EgressProxySource {
    /// 没配代理，出网插件直连。
    #[serde(rename = "none")]
    Unset,
    /// 主人给安装脚本的 `WONTON_EGRESS_PROXY`：设置里不能改，改的话回 409 `conflict`。
    Env,
    /// 主人在 App 里填的。
    Settings,
    /// 安装脚本检测到的，App 里能改。
    Detected,
    /// 用装着的代理插件，见 [`EgressProxySettings::provider`]。
    Plugin,
    /// 比这个 App 新的核心给的来源。
    #[serde(other)]
    Unknown,
}

/// 一次探测的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProbe {
    /// 探测的时间（Unix 毫秒）。
    pub probed_at_ms: u64,
    /// 探测到的问题；为空才算通。
    pub problem: Option<EgressProblem>,
}

/// 经代理出不去的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProblem {
    pub reason: EgressFailure,
    /// 核心写的说明，带上状态码或 Docker 的原文，以及主人下一步该做什么。
    pub message: String,
}

/// 探测失败的原因，也是 `egress_proxy_unavailable` 错误的 `detail`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum EgressFailure {
    /// 出口容器不在运行或者镜像拉不到。挡安装。
    RelayFailed,
    /// 连不上代理地址：被拒绝或者没响应。挡安装。
    ProxyUnreachable,
    /// 连上了，代理回 407 或其他非 2xx。挡安装。
    ProxyRejected,
    /// 代理本身是通的，它连不上探测的网站。不挡安装，状态按不确定显示。
    UpstreamFailed,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

/// 能做出网代理的插件。一台服务器只能装一个。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProxyProvider {
    /// 插件 ID。
    pub id: String,
    pub title: String,
}

/// 配上或取消代理时要重启的一个插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressRestart {
    /// 插件 ID。
    pub id: String,
    pub title: String,
}

/// `PUT /_wonton/api/egress-proxy` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProxyUpdate {
    /// `http://`、`https://` 或 `socks5://`，可以带用户名和密码；空串或空值表示取消代理。和 [`EgressProxySettings::url`]
    /// 给的打码地址一样时表示不改：壳拿不到密码，只改网段时带回来的就是它。
    pub url: Option<String>,
    /// 额外直连的网段，CIDR 写法。和 Docker 网络重叠时回 400 `invalid_input`。
    pub direct_cidrs: Vec<String>,
    /// 主人确认要重启的插件 ID；和核心这时算出来的不一样时回 409 `changed`。只改地址时是空列表。
    pub restarts: Vec<String>,
    /// 用装着的代理插件。这时 `url` 要为空，否则回 400 `invalid_input`；没装代理插件时回 404 `not_installed`。
    #[serde(default)]
    pub use_plugin: bool,
}

/// `PUT /_wonton/api/plugins/{id}/lan-webui` 的请求体，响应是 `PluginDetail`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanWebuiUpdate {
    pub enabled: bool,
}

/// 出网代理设置页的状态，在 `PluginsViewModel.egress` 里。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressViewModel {
    /// 最近取到的设置；还没取到时为空。
    pub current: Option<EgressCurrent>,
    pub loading: bool,
    /// 正在改设置：核心可能在重建插件，要等一会儿。
    pub saving: bool,
    /// 这次改动要重启插件，等主人确认。
    pub confirmation: Option<EgressConfirmation>,
    /// 最近一次取设置失败的原因，这时显示的是上次取到的；再取成功后清空。
    pub load_problem: Option<PluginProblem>,
    /// 最近一次改设置失败的原因。再打开这一页或者再保存时清空。
    pub save_problem: Option<PluginProblem>,
}

/// 核心给的设置，加上壳要渲染的结论。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressCurrent {
    pub settings: EgressProxySettings,
    /// 地址输入框的内容：密码打码，没配代理时是空串。原样保存时地址不变。
    pub address: String,
    /// 地址由 `WONTON_EGRESS_PROXY` 指定，这里不能改；网段还能改。
    pub address_locked: bool,
    /// 选用已安装的代理插件；是否可选由 `settings.provider` 决定，也受 `address_locked` 限制。
    pub use_plugin: bool,
    /// 直连网段输入框的内容。
    pub direct_cidrs: String,
    /// 插件的路径；没配代理时为空。
    pub plugin_path: Option<EgressPathState>,
    /// 核心自己的路径；没配代理时为空。
    pub core_path: Option<EgressPathState>,
}

/// 一条路径通不通。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EgressPathState {
    /// 还没有探测结果。
    Probing,
    Reachable,
    /// 代理本身是通的，或者原因比这个 App 新：说不准，不算失败。
    Uncertain {
        message: String,
    },
    Failed {
        message: String,
    },
}

/// 第一次配上或彻底取消代理前要主人点头的内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressConfirmation {
    pub restarts: Vec<EgressRestart>,
    /// 是彻底取消，不是第一次配上。
    pub cancelling: bool,
    /// 主人确认后发给核心的内容。
    pub update: EgressProxyUpdate,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    fn settings() -> EgressProxySettings {
        EgressProxySettings {
            url: Some("http://user:***@172.17.0.1:7899".into()),
            source: EgressProxySource::Settings,
            provider: Some(EgressProxyProvider {
                id: "app.example.proxy".into(),
                title: "代理".into(),
            }),
            plugin_path: Some(EgressProbe {
                probed_at_ms: 1_793_000_000_000,
                problem: Some(EgressProblem {
                    reason: EgressFailure::ProxyUnreachable,
                    message: "5 秒没响应：宿主防火墙要放行 Docker 网段到这个端口".into(),
                }),
            }),
            core_path: Some(EgressProbe {
                probed_at_ms: 1_793_000_000_000,
                problem: None,
            }),
            host_cidrs: vec!["192.168.0.0/24".into()],
            direct_cidrs: vec!["100.64.0.0/10".into()],
            notices: vec!["宿主所在网段 172.18.0.0/16 和 Docker 网络重叠，没有放行".into()],
            restarts: vec![EgressRestart {
                id: "app.hermes".into(),
                title: "Hermes".into(),
            }],
        }
    }

    /// 契约：管理接口的类型都能经 JSON 往返。
    #[test]
    fn egress_types_round_trip_through_json() {
        assert_eq!(round_trip(&settings()), settings());
        let update = EgressProxyUpdate {
            url: Some("socks5://172.17.0.1:7898".into()),
            direct_cidrs: vec!["100.64.0.0/10".into()],
            restarts: vec!["app.hermes".into()],
            use_plugin: false,
        };
        assert_eq!(round_trip(&update), update);
        let plugin = EgressProxyUpdate {
            url: None,
            use_plugin: true,
            ..update
        };
        assert_eq!(round_trip(&plugin), plugin);
        let toggle = LanWebuiUpdate { enabled: true };
        assert_eq!(round_trip(&toggle), toggle);
    }

    /// 契约：核心和客户端核心之间的 JSON 写法。ADR-0023 和 ADR-0039 写明了来源和各个原因的名字。
    #[test]
    fn egress_wire_format_is_stable() {
        for (source, text) in [
            (EgressProxySource::Unset, "none"),
            (EgressProxySource::Env, "env"),
            (EgressProxySource::Settings, "settings"),
            (EgressProxySource::Detected, "detected"),
            (EgressProxySource::Plugin, "plugin"),
        ] {
            assert_eq!(serde_json::to_value(source).unwrap(), json!(text));
        }
        for (failure, text) in [
            (EgressFailure::RelayFailed, "relay_failed"),
            (EgressFailure::ProxyUnreachable, "proxy_unreachable"),
            (EgressFailure::ProxyRejected, "proxy_rejected"),
            (EgressFailure::UpstreamFailed, "upstream_failed"),
        ] {
            assert_eq!(serde_json::to_value(failure).unwrap(), json!(text));
        }
        let cancel: EgressProxyUpdate = serde_json::from_value(json!({
            "url": null,
            "direct_cidrs": [],
            "restarts": ["app.hermes"],
        }))
        .unwrap();
        assert_eq!(cancel.url, None);
        assert!(!cancel.use_plugin, "不写 use_plugin 就是不用代理插件");
        let toggle: LanWebuiUpdate = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert!(toggle.enabled);
    }

    /// 核心可能比 App 新：不认识的取值落到兜底的变体，多出来的字段被忽略，整份设置照样能读。
    #[test]
    fn egress_values_from_a_newer_core_still_parse() {
        let mut value = serde_json::to_value(settings()).unwrap();
        value["source"] = json!("mdm_profile");
        value["plugin_path"]["problem"]["reason"] = json!("quota_exceeded");
        value["added_in_a_later_version"] = json!(true);
        let parsed: EgressProxySettings = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.source, EgressProxySource::Unknown);
        assert_eq!(
            parsed.plugin_path.unwrap().problem.unwrap().reason,
            EgressFailure::Other
        );
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
