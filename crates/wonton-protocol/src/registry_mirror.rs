//! 镜像加速：核心经加速地址按摘要拉 Docker Hub 的镜像（ADR-0040）。
//!
//! 核心的管理接口 `/_wonton/api/registry-mirror` 收发这里的类型，客户端核心原样交给壳。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，不让整份设置解析失败。

use serde::{Deserialize, Serialize};

use crate::PluginProblem;

/// 镜像加速设置页；请求失败时保留最近取到的设置和测速结果。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorViewModel {
    pub settings: Option<RegistryMirrorSettings>,
    pub loading: bool,
    pub saving: bool,
    pub testing: bool,
    pub load_problem: Option<PluginProblem>,
    pub save_problem: Option<PluginProblem>,
    pub test_problem: Option<PluginProblem>,
    pub tests: Option<RegistryMirrorTests>,
}

/// `GET /_wonton/api/registry-mirror` 和 `PUT /_wonton/api/registry-mirror` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorSettings {
    /// 在用的加速地址（主机名，可以带端口）；没打开时为空。
    pub mirror: Option<String>,
    /// 官方索引给的预置地址，按索引里的顺序；索引读不到时为空，这时只能自己填。
    pub choices: Vec<String>,
    /// `choices` 读不懂（或整个索引读不到）时核心写的整句，壳显示在表单上；读到了就没有。
    pub choices_problem: Option<String>,
    /// 最近一次直连 Docker Hub 的结果；还没测过，或者 Docker 守护进程自己配了代理（这时核心测的路径不是它拉镜像的路径）
    /// 时为空。
    pub docker_hub: Option<RegistryProbe>,
    /// Docker Hub 直连不通或者太慢，推荐打开。核心不替主人打开。
    pub recommended: bool,
}

/// `PUT /_wonton/api/registry-mirror` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorUpdate {
    /// 加速地址，主机名可以带端口，不写协议（只走 HTTPS）；为空表示关掉。不必在 `choices` 里。
    /// 写法不对时回 400 `invalid_input`。
    pub mirror: Option<String>,
}

/// `POST /_wonton/api/registry-mirror/test` 的请求体：测这几个地址，同时再测一次 Docker Hub 直连。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorTestRequest {
    pub mirrors: Vec<String>,
}

/// `POST /_wonton/api/registry-mirror/test` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorTests {
    /// 为空的情况同 [`RegistryMirrorSettings::docker_hub`]。
    pub docker_hub: Option<RegistryProbe>,
    /// 和请求里的顺序一样。
    pub mirrors: Vec<RegistryMirrorTest>,
}

/// 一个加速地址的测速结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryMirrorTest {
    pub mirror: String,
    pub probe: RegistryProbe,
}

/// 按摘要取一个固定小镜像的清单的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryProbe {
    /// 测的时间（Unix 毫秒）。
    pub probed_at_ms: u64,
    /// 取到清单用了多久；没取到时为空。
    pub latency_ms: Option<u32>,
    /// 为空才算能用。
    pub problem: Option<RegistryProblem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegistryProblem {
    pub reason: RegistryFailure,
    /// 核心写的说明，带上状态码或者网络错误的原文。
    pub message: String,
}

/// 取不到清单的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RegistryFailure {
    /// 解析不到、连不上、TLS 出错或者超时。
    Unreachable,
    /// 连上了，回了 401、403、429 或者 5xx：要登录、有白名单、限流或者后端坏了。
    Rejected,
    /// 给的内容和摘要对不上。
    DigestMismatch,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    fn probe(problem: Option<RegistryProblem>) -> RegistryProbe {
        RegistryProbe {
            probed_at_ms: 1_793_000_000_000,
            latency_ms: problem.is_none().then_some(780),
            problem,
        }
    }

    /// 契约：管理接口的类型都能经 JSON 往返。
    #[test]
    fn registry_mirror_types_round_trip_through_json() {
        let settings = RegistryMirrorSettings {
            mirror: Some("mirror.example.com".into()),
            choices: vec!["mirror.example.com".into(), "hub.example.net:5000".into()],
            choices_problem: Some("索引里的 registry_mirrors 读不懂".into()),
            docker_hub: Some(probe(Some(RegistryProblem {
                reason: RegistryFailure::Unreachable,
                message: "5 秒没响应".into(),
            }))),
            recommended: true,
        };
        assert_eq!(round_trip(&settings), settings);
        let update = RegistryMirrorUpdate { mirror: None };
        assert_eq!(round_trip(&update), update);
        let request = RegistryMirrorTestRequest {
            mirrors: vec!["mirror.example.com".into()],
        };
        assert_eq!(round_trip(&request), request);
        let tests = RegistryMirrorTests {
            docker_hub: Some(probe(None)),
            mirrors: vec![RegistryMirrorTest {
                mirror: "mirror.example.com".into(),
                probe: probe(Some(RegistryProblem {
                    reason: RegistryFailure::Rejected,
                    message: "403".into(),
                })),
            }],
        };
        assert_eq!(round_trip(&tests), tests);
    }

    /// 契约：原因的线上写法。
    #[test]
    fn registry_failure_wire_format_is_stable() {
        for (failure, text) in [
            (RegistryFailure::Unreachable, "unreachable"),
            (RegistryFailure::Rejected, "rejected"),
            (RegistryFailure::DigestMismatch, "digest_mismatch"),
        ] {
            assert_eq!(serde_json::to_value(failure).unwrap(), json!(text));
        }
    }

    /// 核心可能比 App 新：不认识的原因落到 `Other`，多出来的字段被忽略。
    #[test]
    fn registry_values_from_a_newer_core_still_parse() {
        let value = json!({
            "mirror": null,
            "choices": [],
            "docker_hub": {"probed_at_ms": 1, "latency_ms": null, "problem": {"reason": "quota", "message": "x"}},
            "recommended": false,
            "added_in_a_later_version": true,
        });
        let parsed: RegistryMirrorSettings = serde_json::from_value(value).unwrap();
        assert_eq!(
            parsed.docker_hub.unwrap().problem.unwrap().reason,
            RegistryFailure::Other
        );
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
