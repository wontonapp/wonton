use serde::{Deserialize, Serialize};

// Client resources live in wonton-client; keep protocol independent of its catalog and dependencies.
static DETAIL_FORMATTER: std::sync::OnceLock<fn(&str, &str) -> String> = std::sync::OnceLock::new();

/// 客户端在创建核心前安装一次；不改变错误的字段或原始原因。
pub fn set_core_error_formatter(formatter: fn(&str, &str) -> String) {
    let _ = DETAIL_FORMATTER.set(formatter);
}

fn detail(id: &str, reason: &str) -> String {
    if let Some(formatter) = DETAIL_FORMATTER.get() {
        return formatter(id, reason);
    }
    // 未加载客户端目录时保留消息 id 和原因，协议 crate 不持有客户端文案。
    format!("{id}: {reason}")
}

/// 核心对外返回的错误。每个变体对应一种原因，壳据此决定怎么提示，不把不同原因混成一种。
///
/// 后台进行的网络操作不走这里：连接家里服务器的进展和失败原因体现在 `HomeViewModel` 里，聊天的体现在
/// `ChatViewModel` 和时间线里。直接返回结果的调用（如取媒体）按 AGENTS.md 的分类返回下面的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi::export(Display))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreError {
    /// 操作的参数不满足要求，或引用了不存在的对象。
    #[error("{reason}")]
    InvalidInput { reason: String },
    /// 读写本机存储失败（设备密钥、连接信息、聊天会话、外观设置）。
    #[error("{}", detail("core-error-storage", .reason))]
    Storage { reason: String },
    /// 前置条件不满足：聊天还没登录、房间还没同步到本机等。
    #[error("{}", detail("core-error-not-ready", .reason))]
    NotReady { reason: String },
    /// 连不上家里的服务器或它上面的服务：隧道没连上、网络断开或请求超时。
    #[error("{}", detail("core-error-transport", .reason))]
    TransportUnavailable { reason: String },
    /// 服务器不认这个身份：设备已被吊销、令牌失效。要重新配对或登录。
    #[error("{}", detail("core-error-unauthorized", .reason))]
    Unauthorized { reason: String },
    /// 身份有效，但服务器不让做这件事：没有权限、不在房间里、对方屏蔽了自己、对方的服务器不和这台互通。不是要重新登录。
    #[error("{}", detail("core-error-forbidden", .reason))]
    Forbidden { reason: String },
    /// 服务器上的服务出错。
    #[error("{}", detail("core-error-upstream", .reason))]
    Upstream { reason: String },
    /// 服务器比 App 旧，还没有这个接口（[`crate::minimum_server_version`]）。升级服务器后可用；服务器不旧却回
    /// 404 时不归这一类，照旧按原来的类别报。
    #[error("{}", detail("core-error-server-too-old", .reason))]
    ServerTooOld {
        reason: String,
        /// 这个接口要的服务器版本；不知道时为空。
        min_version: Option<String>,
    },
    /// 插件设置没过校验（ADR-0077）：逐字段的明细给壳标到控件旁，`reason` 是核心写的一句话。
    #[error("{reason}")]
    SettingsInvalid {
        reason: String,
        /// 没通过的字段，键是属性名。
        fields: Vec<crate::SettingsFieldError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_too_old_round_trips_with_its_minimum() {
        let error = CoreError::ServerTooOld {
            reason: "0.1.0-beta.4 没有这个接口".to_owned(),
            min_version: Some("0.1.0-beta.5".to_owned()),
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["type"], "server_too_old");
        assert_eq!(serde_json::from_value::<CoreError>(json).unwrap(), error);
    }

    #[test]
    fn server_too_old_without_a_known_minimum_is_fine() {
        let error = CoreError::ServerTooOld {
            reason: "太旧".to_owned(),
            min_version: None,
        };
        assert!(serde_json::from_value::<CoreError>(serde_json::to_value(&error).unwrap()).is_ok());
    }

    #[test]
    fn settings_invalid_round_trips_with_its_fields() {
        let error = CoreError::SettingsInvalid {
            reason: "1 settings don't pass validation".to_owned(),
            fields: vec![crate::SettingsFieldError {
                name: "months".to_owned(),
                kind: crate::SettingsFieldProblemKind::AboveMaximum,
                message: "Must be at most 12".to_owned(),
            }],
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["type"], "settings_invalid");
        assert_eq!(serde_json::from_value::<CoreError>(json).unwrap(), error);
    }
}
