//! 插件的持久化运行意图与中断恢复日志；与容器当前观测状态分开。

use serde::{Deserialize, Serialize};

/// 主人最后明确要求的运行状态；旧记录没有意图时只观察，不自动启动。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginDesiredState {
    #[default]
    Unspecified,
    Running,
    Stopped,
}

/// 中断操作最后持久化的阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginRecoveryStage {
    Preparing,
    Replacing,
    Starting,
    Reconciling,
    Stopping,
}

/// 恢复日志：类别与 InstallError 一致，进程中断使用 interrupted。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginRecovery {
    pub category: String,
    pub reason: String,
    pub stage: PluginRecoveryStage,
    pub attempts: u32,
    /// Unix 秒；空表示正在执行或重试额度已耗尽。
    pub next_retry_at: Option<u64>,
}

/// GET /_wonton/api/plugins/{id}/lifecycle 的响应。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginLifecycle {
    pub desired: PluginDesiredState,
    pub recovery: Option<PluginRecovery>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_observe_only_without_recovery() {
        let lifecycle = PluginLifecycle::default();
        assert_eq!(lifecycle.desired, PluginDesiredState::Unspecified);
        assert_eq!(lifecycle.recovery, None);
        assert_eq!(
            serde_json::to_value(lifecycle).unwrap(),
            serde_json::json!({"desired": "unspecified", "recovery": null})
        );
    }

    #[test]
    fn interrupted_recovery_preserves_retry_budget_on_the_wire() {
        let value = serde_json::json!({
            "desired": "running",
            "recovery": {
                "category": "interrupted",
                "reason": "core restarted while replacing containers",
                "stage": "replacing",
                "attempts": 4,
                "next_retry_at": 1_800_000_000_u64
            }
        });
        let lifecycle: PluginLifecycle = serde_json::from_value(value.clone()).unwrap();
        let recovery = lifecycle.recovery.as_ref().unwrap();
        assert_eq!(recovery.stage, PluginRecoveryStage::Replacing);
        assert_eq!(recovery.attempts, 4);
        assert_eq!(recovery.next_retry_at, Some(1_800_000_000));
        assert_eq!(serde_json::to_value(lifecycle).unwrap(), value);
    }
}
