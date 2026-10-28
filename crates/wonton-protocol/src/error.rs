use serde::{Deserialize, Serialize};

/// 核心对外返回的错误。每个变体对应一种原因，壳据此决定怎么提示，不把不同原因混成一种。
///
/// 目前只有输入非法；接入网络、权限和插件后，再按 AGENTS.md 的分类增加变体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi::export(Display))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreError {
    /// 操作的参数不满足要求，或引用了不存在的对象。
    #[error("输入非法：{reason}")]
    InvalidInput { reason: String },
}
