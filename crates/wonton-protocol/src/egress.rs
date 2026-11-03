//! 出网插件走宿主的代理（ADR-0023 的“接口：App 和核心”）。
//!
//! 核心的管理接口 `/_wonton/api/egress-proxy` 收发这里的类型，客户端核心原样交给壳。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other` 或 `Unknown`，不让整份设置解析失败。

use serde::{Deserialize, Serialize};

/// `GET /_wonton/api/egress-proxy` 和 `PUT /_wonton/api/egress-proxy` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EgressProxySettings {
    /// 主人的代理地址，密码打码；没配代理时为空。
    pub url: Option<String>,
    pub source: EgressProxySource,
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
}

/// `PUT /_wonton/api/plugins/{id}/lan-webui` 的请求体，响应是 `PluginDetail`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanWebuiUpdate {
    pub enabled: bool,
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
        };
        assert_eq!(round_trip(&update), update);
        let toggle = LanWebuiUpdate { enabled: true };
        assert_eq!(round_trip(&toggle), toggle);
    }

    /// 契约：核心和客户端核心之间的 JSON 写法。ADR-0023 写明了 `none`、`env`、`settings`、`detected` 和各个原因的名字。
    #[test]
    fn egress_wire_format_is_stable() {
        for (source, text) in [
            (EgressProxySource::Unset, "none"),
            (EgressProxySource::Env, "env"),
            (EgressProxySource::Settings, "settings"),
            (EgressProxySource::Detected, "detected"),
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
