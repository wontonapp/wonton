//! 按钮触发的一次动作跑到了哪一步（ADR-0026 的“动作按钮的反馈”）。
//!
//! 进行中和结果都由核心给：壳按它画按钮（进行中转圈、不能再点），没做成时按类别写一句人话，
//! 原文收进默认收起的详情。

use serde::{Deserialize, Serialize};

/// 一次动作的进度和结果。壳不自己记点没点过、成没成。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionRun {
    /// 没在做：按钮照常可点。
    #[default]
    Idle,
    /// 核心在做：按钮转圈、不能再点。
    Running,
    /// 做成了，结果在别处看不出来时才用；看得出来的（页面加载出来了）直接回 [`ActionRun::Idle`]。
    Succeeded,
    /// 没做成。`details` 是给愿意看的人的原文（错误码、地址收在这里），`None` 表示没有。
    Failed {
        failure: ActionFailure,
        #[serde(default)]
        details: Option<String>,
    },
    /// 等不到结果：不知道做成了没有，如实说不确定。
    Uncertain,
    /// 比这个 App 新的取值：照 [`ActionRun::Idle`] 画，不当成功也不当失败。
    #[serde(other)]
    Other,
}

impl ActionRun {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// 没做成的原因类别，按 AGENTS.md 的分法；说法在壳里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ActionFailure {
    /// 连不上家里的服务器。
    Transport,
    /// 没有权限，或这台设备不被认了。
    Permission,
    /// 插件没装或还没就绪。
    NotReady,
    /// 家里的服务器或插件出错。
    Upstream,
    /// 做了，但还是老样子：比如重试以后插件仍然出错。
    StillFailing,
    /// 输入不合法，或要操作的东西已经不在了。
    InvalidInput,
    /// 比这个 App 新的类别。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_runs_and_failures_fall_back_to_other() {
        let run: ActionRun = serde_json::from_value(serde_json::json!({"type": "queued"})).unwrap();
        assert_eq!(run, ActionRun::Other);
        let run: ActionRun = serde_json::from_value(serde_json::json!({
            "type": "failed", "failure": "rate_limited"
        }))
        .unwrap();
        assert_eq!(
            run,
            ActionRun::Failed {
                failure: ActionFailure::Other,
                details: None
            }
        );
    }

    #[test]
    fn a_failed_run_round_trips_with_its_details() {
        let run = ActionRun::Failed {
            failure: ActionFailure::StillFailing,
            details: Some("container exited (1)".into()),
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(serde_json::from_value::<ActionRun>(json).unwrap(), run);
    }
}
