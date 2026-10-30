use serde::{Deserialize, Serialize};

use crate::{ImageUpload, PushEndpoint, SecretString};

/// 原生壳发给客户端核心的用户操作。
///
/// 壳只负责把用户手势翻译成操作；操作是否合法、状态怎么变，都由核心决定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// 新建一个会话并选中它。标题去掉首尾空白后不能为空。
    CreateSession { title: String },
    /// 选中一个已有会话。
    SelectSession { session_id: String },
    /// 删除一个会话；删除的是选中的会话时，清空选中。
    DeleteSession { session_id: String },
    /// 连接家里的服务器，并把连接信息存在本机，之后每次启动自动连接。M0 的连接信息由开发脚本给出
    /// （`deploy/dev/core/dev.sh ticket`），配对上线后来自二维码（#16）。连接信息不合法时返回
    /// `InvalidInput`；连接的进展和结果体现在 [`HomeViewModel`](crate::HomeViewModel) 里。
    ConnectHome { ticket: String },
    /// 扫码配对：`code` 是二维码里的文字（`wonton://pair?…`，格式见 ADR-0009 的“接口：扫码配对”），里面有一次性的
    /// 配对密钥，所以和恢复密钥一样打码；`device_name` 是主人在设备列表里认这台设备用的名字。不是配对码时返回
    /// `InvalidInput`；进展和结果在 [`HomeViewModel::pairing`](crate::HomeViewModel::pairing) 里，配对成功后存下连接
    /// 信息并连上，之后每次启动自动连接，和 `ConnectHome` 一样。
    PairHome {
        code: SecretString,
        device_name: String,
    },
    /// “添加设备”：经隧道向家里的服务器要一个一次性配对码，显示给新设备扫（ADR-0009 的“接口：扫码配对”）。还没配对
    /// 时返回 `NotReady`；进展和码在 [`HomeViewModel::invite`](crate::HomeViewModel::invite) 里。再发一次换一个新码，
    /// 旧码到期前仍然能用。
    InviteDevice,
    /// App 回到前台。核心检查隧道和本地代理是否还能用，必要时重建；系统挂起 App 期间可能回收了套接字。
    /// Matrix 同步停了的话也在这时重启。
    AppForegrounded,

    // 聊天（Matrix）。网络操作都在后台执行：进展体现在 `ChatViewModel` 和时间线里，失败放进
    // `ChatViewModel::problem`。参数不合法、还没登录时直接返回错误。
    /// 接受房间邀请。
    JoinRoom { room_id: String },
    /// 拒绝邀请，或者离开房间。
    LeaveRoom { room_id: String },
    /// 和另一个用户（比如 Bot）发起私聊；已经有和他的私聊时什么都不做。
    StartDirectChat { user_id: String },
    /// 发送文字。正文去掉首尾空白后不能为空。消息先以本地回显出现在时间线里。
    SendText { room_id: String, body: String },
    /// 发送图片。上传和发送的进展同 [`Action::SendText`]。
    SendImage { room_id: String, image: ImageUpload },
    /// 加载更早的消息。要先订阅这个房间的时间线，结果体现在时间线的增量和翻页状态里。
    PaginateBackwards { room_id: String },
    /// 把房间标为读到最新一条，发出已读回执，房间的未读数随之清零。
    MarkRoomRead { room_id: String },
    /// 设置恢复：必要时先建交叉签名身份，再建密钥备份和秘密存储，生成恢复密钥，放进
    /// `EncryptionStatus::recovery_key` 给主人抄下。
    SetUpRecovery,
    /// 主人已经把恢复密钥另外保存：核心把它从快照和本机清掉，之后不再显示。
    ConfirmRecoveryKeySaved,
    /// 用恢复密钥恢复：取回交叉签名的秘密和密钥备份，本设备随之被验证，能解密历史消息。
    RecoverEncryption { recovery_key: SecretString },
    /// 清掉 `ChatViewModel::problem`。
    DismissChatProblem,

    // 推送（ADR-0003 的“设计：加密推送”）。进展在 `ChatViewModel::push` 里。
    /// 平台给了（或者换了）推送地址。核心经隧道把它登记给家里的服务器，再在 Matrix 上注册指向家里推送代理的
    /// pusher；之后每次登录都重新注册一遍。地址或公钥不合法时返回 `InvalidInput`。
    SetPushEndpoint { endpoint: PushEndpoint },
    /// 平台收不了推送了：没有装分发器，分发器拒绝了登记或者注销了本 App。核心删掉家里的登记和 pusher，
    /// 把 `message` 照实放进 `ChatViewModel::push`。
    PushUnavailable { message: String },
}
