package app.wonton.ui

import app.wonton.protocol.BotRef

/** 点通知的目标：聊天房间、旁聊、Bot 联系人页，或没有房间的确认卡片列表。 */
data class RoomRequest(
    val serverId: String,
    val roomId: String,
    val contacts: BotRef? = null,
    val cardId: String? = null,
    val sideChatRoot: String? = null,
)
