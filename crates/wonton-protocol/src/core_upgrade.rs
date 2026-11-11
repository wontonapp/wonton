//! 核心的一键升级（ADR-0025）。
//!
//! 核心的管理接口 [`CORE_UPGRADE_PATH`] 收发这里的类型，客户端核心再加上断开期间的判断，做成
//! [`CoreUpgradeViewModel`] 交给壳。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，不让整份状态解析失败。

use serde::{Deserialize, Serialize};

/// `GET` 取状态，`POST` 发起升级（[`CoreUpgradeRequest`]），`DELETE` 请求取消（回 202）。三个都带 [`CoreUpgradeStatus`]。
/// 被拒绝时的错误类型是 `changed`、`not_ready`、`busy`、`too_late`，见 [`CoreUpgradeFailure`] 的同名取值。
pub const CORE_UPGRADE_PATH: &str = "/_wonton/api/core/upgrade";

/// `POST` 马上查一次新版本（[`CoreUpgradeCheckRequest`]），回 [`CoreUpgradeStatus`]，结果在 `check` 里。
/// 有没结束的升级时不查，直接回当前状态，不算错误。
pub const CORE_UPGRADE_CHECK_PATH: &str = "/_wonton/api/core/upgrade/check";

/// 核心的管理接口返回的升级状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeStatus {
    /// 正在运行的版本。
    pub current: CoreVersion,
    /// 跟的渠道：`WONTON_CORE_SOURCE` 的 tag，如 `stable`。不跟渠道时为空。
    pub channel: Option<String>,
    pub check: CoreUpgradeCheck,
    /// 这次或上次升级；没有状态文件时为空。
    pub run: Option<CoreUpgradeRun>,
}

/// 核心的一个版本。digest 是版本的身份（ADR-0012），版本号只给人看。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreVersion {
    /// 镜像的 `org.opencontainers.image.version` 标签，如 `0.3.0`。开发版的镜像没有。
    pub version: Option<String>,
    /// `sha256:…`。本机构建、没推过仓库的镜像没有。
    pub digest: Option<String>,
}

/// 渠道上有没有新版本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CoreUpgradeCheck {
    /// 这个核心不是正式安装的，不能一键升级。
    NotInstalled,
    /// 这次安装固定在一个 digest 上，不跟渠道。
    Pinned,
    /// 启动后还没查过。
    NotChecked,
    /// 第一次查，还没有结果。复查期间保留上次的结果，不变成这个，否则确认页上的新版本会消失。
    Checking,
    /// 已经是渠道上的最新版。
    UpToDate,
    /// 渠道上有新的 digest，正在下载。下载好才知道版本号。
    Downloading {
        digest: String,
        /// 已经下载的字节数和总字节数。还没拿到各层大小时为空。
        progress: Option<CoreUpgradeProgress>,
    },
    /// 新版本下载好了，可以升级。
    Ready { offer: CoreUpgradeOffer },
    /// 渠道上有新的 digest，但没下载下来：拉不到，或者磁盘不够。主人可以再查一次。
    DownloadFailed {
        digest: String,
        problem: CoreUpgradeProblem,
    },
    /// 没查成：连不上镜像仓库，或者渠道的 tag 不存在。
    Failed { problem: CoreUpgradeProblem },
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 下载进展。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// 可以升级到的版本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeOffer {
    /// 发起升级时带回去。
    pub digest: String,
    pub version: Option<String>,
    /// 这次升级会迁移、之后退不回去的卷，如 `tuwunel`（ADR-0025 的“不可逆的版本”）。不为空时确认页要醒目提示。
    pub migrates: Vec<String>,
}

/// 一次升级。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeRun {
    /// 这次升级的编号。
    pub id: String,
    pub from: CoreVersion,
    pub to: CoreUpgradeTarget,
    pub state: CoreUpgradeRunState,
}

/// 升级的目标。发起升级必须带 digest，所以这里总有：判断“已完成”靠它和正在运行的 digest 比，两边都空不能算相等。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeTarget {
    pub digest: String,
    pub version: Option<String>,
}

/// 升级的进展和结果。名字和状态文件里的结果一一对应，含义见 ADR-0025 的“步骤”。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CoreUpgradeRunState {
    InProgress {
        phase: CoreUpgradePhase,
        /// 到回滚结束的最长剩余秒数（各阶段上限加余量）。连接断了以后，客户端核心最多等这么久才显示“不确定”。
        remaining_secs: u64,
    },
    /// 升级成功。
    Completed,
    /// 没升级成，退回了旧版本。
    RolledBack { problem: CoreUpgradeProblem },
    /// 没升级成，什么都没动。
    Failed { problem: CoreUpgradeProblem },
    /// 主人取消了，什么都没动。
    Canceled,
    /// 回滚也失败了，系统的状态说不清。
    RollbackFailed { problem: CoreUpgradeProblem },
    /// 不可逆的版本失败了，停在新版本上，没有自动回滚。
    Unresolved { problem: CoreUpgradeProblem },
    /// 升级器不在了，说不清做到哪一步。
    Unknown { problem: CoreUpgradeProblem },
    /// 比这个 App 新的核心给的结果：按“不确定”算，不算成功。
    #[serde(other)]
    Other,
}

/// 升级做到哪一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CoreUpgradePhase {
    /// 拉镜像。什么都没动，还能取消。
    Pulling,
    BackingUp,
    Switching,
    Starting,
    /// 过了健康检查，再观察几分钟。
    Observing,
    RollingBack,
    #[serde(other)]
    Other,
}

/// 和升级有关的操作失败了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeProblem {
    pub reason: CoreUpgradeFailure,
    /// 给主人看的说明，放在详情里：带上镜像、服务的名字或者上游的原话。
    pub message: String,
}

/// 失败的原因，壳据此提示主人下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CoreUpgradeFailure {
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 家里的服务器不认这台设备，或者不让它做这件事。
    Unauthorized,
    /// 渠道上的版本在主人确认之后变了：要重新确认。
    Changed,
    /// 新版本还没下载好。
    NotReady,
    /// 已经在升级了。
    Busy,
    /// 已经开始动东西了，不能取消。
    TooLate,
    /// 连不上镜像仓库，或者仓库里没有这个镜像。
    RegistryUnavailable,
    /// 磁盘不够。
    NoSpace,
    /// 新镜像的 `init` 拒绝了或者失败了，比如要先升到某个版本。
    InitFailed,
    /// 新版本没通过健康检查。
    Unhealthy,
    /// 新版本过了健康检查，观察期里崩了。
    Crashed,
    /// 超过了时限。
    TimedOut,
    /// 升级器不在了，或者反复崩溃。
    UpdaterFailed,
    /// Docker 出错了。
    Upstream,
    /// App 和家里服务器的版本对不上，有一边要更新。
    Protocol,
    #[serde(other)]
    Other,
}

/// 发起升级的请求体。镜像仓库由家里服务器的配置决定，请求指定不了别的镜像。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreUpgradeRequest {
    /// 主人确认的 [`CoreUpgradeOffer::digest`]。
    pub digest: String,
}

/// 查新版本的请求体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreUpgradeCheckRequest {
    /// 主人点了“检查”：下载时按镜像大小加余量看磁盘。否则是打开升级页顺带查，磁盘剩不到 5 GB 就不下载（ADR-0025）。
    pub manual: bool,
}

/// 客户端核心给壳的升级页。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CoreUpgradeViewModel {
    /// 最后一次从家里的服务器取到的状态；还没取到过时为空。
    pub status: Option<CoreUpgradeStatus>,
    /// 壳显示什么。只有这里说 [`CoreUpgradeStage::Completed`] 才算升级成功。
    pub stage: CoreUpgradeStage,
    /// 现在能升级到的版本。在升级时为空，这时 `status.check` 还是发起前的结果。
    pub offer: Option<CoreUpgradeOffer>,
    /// 主人在这台设备上跳过了 `offer` 这个版本：不再提示，想升还能升。渠道上出了别的版本时重新提示。
    pub offer_skipped: bool,
    /// 主人点了“检查”，结果还没出来。
    pub refreshing: bool,
    /// 最近一次操作失败的原因，这时 `status` 是上次取到的；下一次操作成功后清空。
    pub problem: Option<CoreUpgradeProblem>,
}

impl CoreUpgradeViewModel {
    /// 有新版本，主人在这台设备上没跳过：设置和底栏上画角标。
    pub fn is_prompting(&self) -> bool {
        self.offer.is_some() && !self.offer_skipped
    }
}

/// 升级走到哪了，ADR-0025 的“App 显示”。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum CoreUpgradeStage {
    /// 没有在升级，也没有上次升级的结果。
    #[default]
    Idle,
    /// 主人点了升级，请求还没得到回应。
    Requesting,
    InProgress {
        phase: CoreUpgradePhase,
        /// 现在点“取消”有用：还在拉镜像，取消请求也还没发出去。
        can_cancel: bool,
    },
    /// 最后一次看到的是升级中，连接断了，还没超过核心给的时限。核心重启时会这样。
    Reconnecting,
    /// 说不清结果：超过了时限，或者核心这样报告。连接恢复后，核心的报告会替换它。
    Uncertain {
        problem: CoreUpgradeProblem,
    },
    Completed {
        to: CoreUpgradeTarget,
    },
    RolledBack {
        to: CoreVersion,
        problem: CoreUpgradeProblem,
    },
    Failed {
        problem: CoreUpgradeProblem,
    },
    Canceled,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn version(version: &str, digest: &str) -> CoreVersion {
        CoreVersion {
            version: Some(version.into()),
            digest: Some(digest.into()),
        }
    }

    fn problem(reason: CoreUpgradeFailure) -> CoreUpgradeProblem {
        CoreUpgradeProblem {
            reason,
            message: "说明".into(),
        }
    }

    /// 接口的线上格式：核心和 App 版本不同步，改了这里两边就对不上。
    #[test]
    fn the_status_has_a_stable_wire_format() {
        let status = CoreUpgradeStatus {
            current: version("0.3.0", "sha256:old"),
            channel: Some("stable".into()),
            check: CoreUpgradeCheck::Ready {
                offer: CoreUpgradeOffer {
                    digest: "sha256:new".into(),
                    version: Some("0.4.0".into()),
                    migrates: vec!["tuwunel".into()],
                },
            },
            run: Some(CoreUpgradeRun {
                id: "u1".into(),
                from: version("0.2.0", "sha256:older"),
                to: CoreUpgradeTarget {
                    digest: "sha256:old".into(),
                    version: Some("0.3.0".into()),
                },
                state: CoreUpgradeRunState::InProgress {
                    phase: CoreUpgradePhase::BackingUp,
                    remaining_secs: 4680,
                },
            }),
        };
        let wire = json!({
            "current": {"version": "0.3.0", "digest": "sha256:old"},
            "channel": "stable",
            "check": {
                "state": "ready",
                "offer": {"digest": "sha256:new", "version": "0.4.0", "migrates": ["tuwunel"]},
            },
            "run": {
                "id": "u1",
                "from": {"version": "0.2.0", "digest": "sha256:older"},
                "to": {"digest": "sha256:old", "version": "0.3.0"},
                "state": {"state": "in_progress", "phase": "backing_up", "remaining_secs": 4680},
            },
        });
        assert_eq!(serde_json::to_value(&status).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<CoreUpgradeStatus>(wire).unwrap(),
            status
        );
    }

    #[test]
    fn every_result_round_trips_under_its_adr_name() {
        let cases = [
            (CoreUpgradeRunState::Completed, "completed"),
            (CoreUpgradeRunState::Canceled, "canceled"),
            (
                CoreUpgradeRunState::RolledBack {
                    problem: problem(CoreUpgradeFailure::Unhealthy),
                },
                "rolled_back",
            ),
            (
                CoreUpgradeRunState::Failed {
                    problem: problem(CoreUpgradeFailure::RegistryUnavailable),
                },
                "failed",
            ),
            (
                CoreUpgradeRunState::RollbackFailed {
                    problem: problem(CoreUpgradeFailure::TimedOut),
                },
                "rollback_failed",
            ),
            (
                CoreUpgradeRunState::Unresolved {
                    problem: problem(CoreUpgradeFailure::Crashed),
                },
                "unresolved",
            ),
            (
                CoreUpgradeRunState::Unknown {
                    problem: problem(CoreUpgradeFailure::UpdaterFailed),
                },
                "unknown",
            ),
        ];
        for (state, name) in cases {
            let value = serde_json::to_value(&state).unwrap();
            assert_eq!(value["state"], name);
            assert_eq!(
                serde_json::from_value::<CoreUpgradeRunState>(value).unwrap(),
                state
            );
        }
    }

    /// 枚举都有 `Other` 兜底，改了名字解析不会报错，只会悄悄变成“不认识”。所以逐个固定写法。
    #[test]
    fn the_phases_and_reasons_keep_their_wire_names() {
        let phases = [
            (CoreUpgradePhase::Pulling, "pulling"),
            (CoreUpgradePhase::BackingUp, "backing_up"),
            (CoreUpgradePhase::Switching, "switching"),
            (CoreUpgradePhase::Starting, "starting"),
            (CoreUpgradePhase::Observing, "observing"),
            (CoreUpgradePhase::RollingBack, "rolling_back"),
        ];
        for (phase, name) in phases {
            assert_eq!(serde_json::to_value(phase).unwrap(), json!(name));
            assert_eq!(
                serde_json::from_value::<CoreUpgradePhase>(json!(name)).unwrap(),
                phase
            );
        }
        let reasons = [
            (
                CoreUpgradeFailure::TransportUnavailable,
                "transport_unavailable",
            ),
            (CoreUpgradeFailure::Unauthorized, "unauthorized"),
            (CoreUpgradeFailure::Changed, "changed"),
            (CoreUpgradeFailure::NotReady, "not_ready"),
            (CoreUpgradeFailure::Busy, "busy"),
            (CoreUpgradeFailure::TooLate, "too_late"),
            (
                CoreUpgradeFailure::RegistryUnavailable,
                "registry_unavailable",
            ),
            (CoreUpgradeFailure::NoSpace, "no_space"),
            (CoreUpgradeFailure::InitFailed, "init_failed"),
            (CoreUpgradeFailure::Unhealthy, "unhealthy"),
            (CoreUpgradeFailure::Crashed, "crashed"),
            (CoreUpgradeFailure::TimedOut, "timed_out"),
            (CoreUpgradeFailure::UpdaterFailed, "updater_failed"),
            (CoreUpgradeFailure::Upstream, "upstream"),
            (CoreUpgradeFailure::Protocol, "protocol"),
        ];
        for (reason, name) in reasons {
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
            assert_eq!(
                serde_json::from_value::<CoreUpgradeFailure>(json!(name)).unwrap(),
                reason
            );
        }
    }

    #[test]
    fn every_check_state_keeps_its_wire_format() {
        let failed = json!({"reason": "no_space", "message": "说明"});
        let cases = [
            (
                CoreUpgradeCheck::NotInstalled,
                json!({"state": "not_installed"}),
            ),
            (CoreUpgradeCheck::Pinned, json!({"state": "pinned"})),
            (
                CoreUpgradeCheck::NotChecked,
                json!({"state": "not_checked"}),
            ),
            (CoreUpgradeCheck::Checking, json!({"state": "checking"})),
            (CoreUpgradeCheck::UpToDate, json!({"state": "up_to_date"})),
            (
                CoreUpgradeCheck::Downloading {
                    digest: "sha256:new".into(),
                    progress: Some(CoreUpgradeProgress {
                        done_bytes: 1,
                        total_bytes: 2,
                    }),
                },
                json!({
                    "state": "downloading",
                    "digest": "sha256:new",
                    "progress": {"done_bytes": 1, "total_bytes": 2},
                }),
            ),
            (
                CoreUpgradeCheck::DownloadFailed {
                    digest: "sha256:new".into(),
                    problem: problem(CoreUpgradeFailure::NoSpace),
                },
                json!({"state": "download_failed", "digest": "sha256:new", "problem": failed}),
            ),
            (
                CoreUpgradeCheck::Failed {
                    problem: problem(CoreUpgradeFailure::NoSpace),
                },
                json!({"state": "failed", "problem": failed}),
            ),
        ];
        for (check, wire) in cases {
            assert_eq!(serde_json::to_value(&check).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<CoreUpgradeCheck>(wire).unwrap(),
                check
            );
        }
    }

    /// 比 App 新的核心不能让状态解析失败，新的结果也不能被当成成功。
    #[test]
    fn values_from_a_newer_core_do_not_break_parsing() {
        let status: CoreUpgradeStatus = serde_json::from_value(json!({
            "current": {"version": null, "digest": null, "built_at": "新字段"},
            "channel": null,
            "check": {"state": "paused_by_owner", "until": "明天"},
            "run": {
                "id": "u1",
                "from": {"version": null, "digest": null},
                "to": {"digest": "sha256:new", "version": null},
                "state": {"state": "half_done", "note": "新结果"},
            },
        }))
        .unwrap();
        assert_eq!(status.check, CoreUpgradeCheck::Other);
        assert_eq!(status.run.unwrap().state, CoreUpgradeRunState::Other);

        let phase = json!({"state": "in_progress", "phase": "new_phase", "remaining_secs": 1});
        assert_eq!(
            serde_json::from_value::<CoreUpgradeRunState>(phase).unwrap(),
            CoreUpgradeRunState::InProgress {
                phase: CoreUpgradePhase::Other,
                remaining_secs: 1,
            }
        );
        assert_eq!(
            serde_json::from_value::<CoreUpgradeFailure>(json!("new_reason")).unwrap(),
            CoreUpgradeFailure::Other
        );
    }

    #[test]
    fn the_requests_carry_only_what_the_adr_allows() {
        assert_eq!(
            serde_json::to_value(CoreUpgradeRequest {
                digest: "sha256:new".into()
            })
            .unwrap(),
            json!({"digest": "sha256:new"})
        );
        assert_eq!(
            serde_json::to_value(CoreUpgradeCheckRequest { manual: true }).unwrap(),
            json!({"manual": true})
        );
    }
}
