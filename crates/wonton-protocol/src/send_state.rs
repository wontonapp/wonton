//! 本地回显的发送状态（ADR-0109 第 3 节）和失败类别（第 10 节）。

use serde::{Deserialize, Serialize};

/// 发送状态。别人的消息和从服务器同步来的消息都是 [`SendState::Sent`]。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SendState {
    /// 本地回显，在排队或正在发送。排在失败消息后面时，直到那条发出或取消才继续。
    /// `progress` 是核心合并原图和缩略图后算好的上传比例，范围 `0.0..=1.0`；`None` 表示不确定（文字、排队或总量未知）。
    /// 重试、重发或重启后可能倒退或回到 `None`，壳原样绘制，不自行换算或保持单调。
    /// `Some(1.0)` 不代表发送成功，成功和失败分别由 [`SendState::Sent`] 和 [`SendState::Failed`] 表示。
    Sending {
        progress: Option<f32>,
    },
    Sent,
    /// 发送失败。没发出去的都可以取消（[`TimelineAction::CancelSend`](crate::TimelineAction::CancelSend)）。
    ///
    /// - `recoverable` 为真：连接出了问题（连不上、超时、服务器暂时出错）。核心隔一段时间自动重发，连续失败时间隔
    ///   逐渐拉长，直到发出去；也可以马上重发（[`TimelineAction::RetrySend`](crate::TimelineAction::RetrySend)）。
    /// - 为假：服务器拒收（比如太大、没有权限）。核心不自动重发，同一房间之后发的消息都排在它后面，直到主人重发或者
    ///   取消这一条。
    Failed {
        /// 哪一类失败。说法在壳里，按它写一句话。
        #[serde(default)]
        failure: SendFailure,
        /// 详情：SDK 或家里服务器给的原文，可能带英文。放在失败面板里给愿意看的人，`None` 表示没有。
        /// 旧核心把这个字段叫 `message`，也认。
        #[serde(default, alias = "message")]
        details: Option<String>,
        #[serde(default)]
        recoverable: bool,
    },
}

/// 发送失败的类别。说法在壳里，原文在 [`SendState::Failed::details`]。
///
/// 不认识的值落到 [`SendFailure::Other`]；缺这个字段的旧形状也落到它（[`Default`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SendFailure {
    /// 连不上：网络断了、隧道没通、超时。
    Offline,
    /// 太大：超过家里服务器的上传上限。
    TooLarge,
    /// 这个格式发不了。
    UnsupportedType,
    /// 要回复的那条不在了。
    ReplyTarget,
    /// 家里服务器拒收：没有权限、不在房间里、被屏蔽。
    Rejected,
    /// 家里的服务器出错。
    Upstream,
    /// 别的：更老的核心，或者更新的核心给出的类别。
    #[serde(other)]
    #[default]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sending_progress_has_a_stable_json_contract() {
        for (state, json) in [
            (
                SendState::Sending {
                    progress: Some(0.5),
                },
                serde_json::json!({"type": "sending", "progress": 0.5}),
            ),
            (
                SendState::Sending { progress: None },
                serde_json::json!({"type": "sending", "progress": null}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&state).unwrap(), json);
            assert_eq!(serde_json::from_value::<SendState>(json).unwrap(), state);
        }
        let state: SendState = serde_json::from_str(r#"{"type":"sending"}"#).unwrap();
        assert_eq!(state, SendState::Sending { progress: None });
    }

    /// 契约：没有 `failure` 的旧形状仍然能解析，类别落到 `Other`，原文留在 `details`。
    #[test]
    fn failed_message_of_the_old_shape_becomes_details() {
        let state: SendState =
            serde_json::from_str(r#"{"type":"failed","message":"too big"}"#).unwrap();
        assert_eq!(
            state,
            SendState::Failed {
                failure: SendFailure::Other,
                details: Some("too big".into()),
                recoverable: false,
            }
        );
    }

    /// 契约：没有 `failure`、也没有 `message` 的旧形状仍然能解析，类别落到 `Other`。
    #[test]
    fn failed_without_a_failure_still_parses() {
        let state: SendState = serde_json::from_str(r#"{"type":"failed"}"#).unwrap();
        assert_eq!(
            state,
            SendState::Failed {
                failure: SendFailure::Other,
                details: None,
                recoverable: false,
            }
        );
    }

    /// 契约：每个失败类别的 JSON 名固定（壳按它写说法），类别能原样往返。
    #[test]
    fn failure_categories_round_trip_with_their_names() {
        for (failure, name) in [
            (SendFailure::Offline, "offline"),
            (SendFailure::TooLarge, "too_large"),
            (SendFailure::UnsupportedType, "unsupported_type"),
            (SendFailure::ReplyTarget, "reply_target"),
            (SendFailure::Rejected, "rejected"),
            (SendFailure::Upstream, "upstream"),
            (SendFailure::Other, "other"),
        ] {
            let json = serde_json::Value::from(name);
            assert_eq!(serde_json::to_value(failure).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<SendFailure>(json).unwrap(),
                failure
            );
        }
    }
}
