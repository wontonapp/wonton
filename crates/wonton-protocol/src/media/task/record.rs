//! 长任务记录和建任务的入参。

use serde::{Deserialize, Serialize};

use super::super::Timestamp;
use super::id::{DeviceId, Person, TaskId};
use super::item::FailedItem;
use super::progress::Progress;
use super::state::{TaskKind, TaskState};
use super::target::TaskTarget;

/// 一条长任务。上传与下载共用；`target` 与 `progress.unit` 各自成立。
///
/// 带 `Media` 前缀：Swift 绑定进全局命名空间，`Task` 会和 Swift 自己的 `Task` 撞。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaTask {
    pub id: TaskId,
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub state: TaskState,
    pub progress: Progress,
    pub retry_count: u8,
    pub retry_of: Option<TaskId>,
    /// 下一次自动重试的秒级 Unix 时间；只在 `WaitingNetwork` 时有值，由核心算。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<u64>,
    pub created_at: Timestamp,
    /// 只在上传里有；逐项失败原文
    pub partial_failures: Vec<FailedItem>,
}

/// 建任务用的入参。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskCreate {
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub options: TaskOptions,
}

fn one() -> u8 {
    1
}

/// 建任务时各轴自己的开关（只在传输、只在充电、转码率上限）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskOptions {
    pub wifi_only: bool,
    pub charging_only: bool,
    pub max_bitrate: Option<u32>,
    /// 相册范围（可选）；只传相册不传 items 时由设备枚举
    pub albums: Vec<String>,
    /// 同时在跑几项，1–4
    #[serde(default = "one")]
    pub concurrency: u8,
    /// 立即备份：忽略 wifi/充电条件一次
    pub force_now: bool,
    /// 上传传到哪个媒体源插件；空着按选提供方的规则选。下载跟着条目的 `source` 走，不看这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Default for TaskOptions {
    fn default() -> Self {
        Self {
            wifi_only: false,
            charging_only: false,
            max_bitrate: None,
            albums: Vec::new(),
            concurrency: one(),
            force_now: false,
            source: None,
        }
    }
}
