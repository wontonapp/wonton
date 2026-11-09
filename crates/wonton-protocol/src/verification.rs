//! 新设备由自己账号的另一台设备确认：Matrix 的 SAS 自我验证（ADR-0046）。
//!
//! 两台设备对照同一组表情，主人在两边都确认“相同”后，新设备从旧设备取到交叉签名私钥和备份密钥，以前的消息随之
//! 解开。和输入恢复密钥（[`Action::RecoverEncryption`](crate::Action::RecoverEncryption)）并列，结果一样。

use serde::{Deserialize, Serialize};

/// 正在进行或刚结束的一次设备确认。同一时间只有一次；结束后留在快照里，直到下一次开始，壳据此显示结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VerificationFlow {
    /// 这次确认的 ID，操作里用它引用。
    pub flow_id: String,
    /// 这次是不是本设备发起的：发起的是要被确认的新设备，收到请求的是来确认它的设备。
    pub started_here: bool,
    /// 另一台设备的 Matrix 设备 ID。
    pub other_device_id: String,
    /// 另一台设备的名字（主人配对时起的），没有时为空。
    pub other_device_name: Option<String>,
    pub stage: VerificationStage,
}

/// 一次确认走到哪一步。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerificationStage {
    /// 本设备发起了，等另一台设备上的主人接受。
    Requested,
    /// 另一台设备发来请求，等主人在本设备上接受（[`Action::AcceptDeviceVerification`](crate::Action::AcceptDeviceVerification)）。
    Incoming,
    /// 双方都接受了，正在交换密钥，马上出表情。
    Starting,
    /// 主人对照两台设备上的表情：一样就 [`Action::ConfirmVerification`](crate::Action::ConfirmVerification)，
    /// 不一样就 [`Action::DenyVerification`](crate::Action::DenyVerification)。对方不支持表情时 `emoji` 为空，
    /// 对照三个四位数 `decimals`。
    Compare {
        emoji: Vec<VerificationEmoji>,
        decimals: Vec<u16>,
    },
    /// 本设备已确认相同，等另一台设备确认。
    WaitingForOther,
    /// 双方都确认了。被确认的新设备在取交叉签名私钥和备份密钥，拿齐、备份可用后才到 [`Self::Done`]。
    FetchingKeys,
    /// 完成：新设备已验证，能解开备份里以前的消息。
    Done,
    /// 没有完成。
    Cancelled { reason: VerificationCancel },
    #[serde(other)]
    Other,
}

/// SAS 的一个表情。`index` 是 Matrix 规范 SAS 表情表里的序号（0–63），壳按它取本地化的名字；`symbol` 是表情本身。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VerificationEmoji {
    pub index: u8,
    pub symbol: String,
}

/// 确认没有完成的原因，壳据此提示下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum VerificationCancel {
    /// 主人在本设备上取消或拒绝了。
    ByMe,
    /// 主人在另一台设备上取消或拒绝了。
    ByOther,
    /// 有一边说表情不一样。可能是有人冒充，不要信任这台设备。
    Mismatch,
    /// 对方太久没有回应（Matrix 规定请求 10 分钟过期）。
    Timeout,
    /// 账号里另一台设备接手了这次确认。
    AcceptedElsewhere,
    /// 同一时间又来了一个确认请求。Matrix SDK 遇到这种情况会把两个都取消，防止主人确认错对象：重新发起一次。
    ConcurrentRequest,
    /// 双方确认了，但没从另一台设备取全密钥（它手里没有，或者交出前掉了线），以前的消息还是解不开：再确认一次，
    /// 或者用恢复密钥。
    KeysMissing,
    /// 传输或服务器出错，详情在 `ChatViewModel::problem`。
    Failed,
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    /// 契约：确认的快照能经 JSON 往返。
    #[test]
    fn verification_flows_round_trip_through_json() {
        let stages = [
            VerificationStage::Requested,
            VerificationStage::Incoming,
            VerificationStage::Starting,
            VerificationStage::Compare {
                emoji: vec![VerificationEmoji {
                    index: 0,
                    symbol: "🐶".into(),
                }],
                decimals: vec![1234, 5678, 4321],
            },
            VerificationStage::WaitingForOther,
            VerificationStage::FetchingKeys,
            VerificationStage::Done,
            VerificationStage::Cancelled {
                reason: VerificationCancel::Mismatch,
            },
        ];
        for stage in stages {
            let flow = VerificationFlow {
                flow_id: "flow".into(),
                started_here: true,
                other_device_id: "ABCDEFGH".into(),
                other_device_name: Some("iPhone".into()),
                stage,
            };
            let json = serde_json::to_string(&flow).unwrap();
            assert_eq!(
                serde_json::from_str::<VerificationFlow>(&json).unwrap(),
                flow
            );
        }
    }

    /// 契约：确认的操作能经 JSON 往返；不认识的步骤和原因落到 `Other`，App 和核心分开升级时不出错。
    #[test]
    fn verification_actions_round_trip_and_unknown_values_fall_back() {
        for action in [
            Action::StartDeviceVerification,
            Action::AcceptDeviceVerification {
                flow_id: "flow".into(),
            },
            Action::ConfirmVerification {
                flow_id: "flow".into(),
            },
            Action::DenyVerification {
                flow_id: "flow".into(),
            },
            Action::CancelDeviceVerification {
                flow_id: "flow".into(),
            },
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
        }
        assert_eq!(
            serde_json::from_str::<VerificationStage>(r#"{"type":"qr_scanned"}"#).unwrap(),
            VerificationStage::Other
        );
        assert_eq!(
            serde_json::from_str::<VerificationCancel>(r#""unknown_method""#).unwrap(),
            VerificationCancel::Other
        );
    }
}
