package app.wonton.ui

import app.wonton.protocol.BotRef

/**
 * 点通知要去的地方：[roomId] 的房间，消息发在旁聊里时 [sideChatRoot] 是旁聊的根，进那个旁聊（ADR-0093 §4）；
 * 联系人请求的通知（ADR-0050）没有房间，[contacts] 是被请求的 Bot，去它的联系人页。
 */
data class RoomRequest(
    val serverId: String,
    val roomId: String,
    val contacts: BotRef? = null,
    val sideChatRoot: String? = null,
)
