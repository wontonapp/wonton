use serde::{Deserialize, Serialize};

use crate::{PushEndpoint, PushMethod};

/// 系统推送操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PushAction {
    /// 平台给了（或者换了）推送地址。核心经隧道把它登记给家里的服务器，再在 Matrix 上注册指向家里推送代理的
    /// pusher；之后每次登录都重新注册一遍。地址或公钥不合法时返回 `InvalidInput`。
    SetPushEndpoint { endpoint: PushEndpoint },
    /// 平台收不了推送了：没有装分发器，分发器拒绝了登记或者注销了本 App。核心删掉家里的登记和 pusher，
    /// 把 `message` 照实放进 `ChatViewModel::push`。
    PushUnavailable { message: String },
    /// 换收新消息的方式，存在本机，写不了时返回 `Storage`。换成 [`PushMethod::Keepalive`](crate::PushMethod::Keepalive)
    /// 时核心删掉家里的推送登记和 pusher，同一条消息不会既推送又本地通知；壳不再给推送地址。
    SetPushMethod { method: PushMethod },
}
