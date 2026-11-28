use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 跨服务器配对操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeerAction {
    /// 打开“其他服务器”页面或下拉刷新：经隧道取配对过的服务器列表和出码状态（`GET /_wonton/api/peers`），
    /// 结果在 [`PeersViewModel`](crate::PeersViewModel) 里。
    RefreshPeers,
    /// 打开出码页：取本机长期有效的配对码（`GET /_wonton/api/peers/code`），码在
    /// [`PeersViewModel::code`](crate::PeersViewModel::code) 里，壳画成二维码、放进“分享”。出码页开着时客户端几秒取一次列表，
    /// 好让对方当面扫了以后请求马上出现。
    ShowPeerCode,
    /// 关掉出码页：`PeersViewModel::code` 清空，客户端不再快速取列表。
    HidePeerCode,
    /// 重置配对码（`POST /_wonton/api/peers/code/reset`）：换一个令牌，旧码作废，用旧码发来、还没处理的请求一并清掉。
    ResetPeerCode,
    /// 提交扫来或粘贴的配对码（`wonton://peer?…`）：核心经 `POST /_wonton/api/peers` 交给自己的服务器，向出码方发一条
    /// 配对请求，出码方同意后配对成立。码已作废、对方拒绝过、对方暂时收不了这些原因见
    /// [`PeerError`](crate::PeerError)，体现在 [`PeersViewModel::state`](crate::PeersViewModel::state) 里。
    ScanPeerCode { code: SecretString },
    /// 同意一条发来的请求，`id` 是 [`PeerRequestRow::id`](crate::PeerRequestRow::id)。
    ApprovePeerRequest { id: String },
    /// 拒绝一条发来的请求：记进“已拒绝”，同一台服务器再发来直接回“对方拒绝了”。
    RejectPeerRequest { id: String },
    /// 撤销一次拒绝，`id` 是 [`RejectedPeerRow::id`](crate::RejectedPeerRow::id)：对方再发来照常收下。
    UnrejectPeerRequest { id: String },
    /// 移除和一台服务器的配对，或取消自己发出、对方还没处理的请求（`DELETE /_wonton/api/peers/{server_name}`）：
    /// 两边的名单各自撤掉，已有的跨机房间留着，消息停发。`server_name` 是 [`PeerRow::server_name`](crate::PeerRow::server_name)。
    RemovePeer { server_name: String },
}
