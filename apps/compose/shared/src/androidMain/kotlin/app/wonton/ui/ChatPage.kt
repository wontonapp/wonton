package app.wonton.ui

import app.wonton.protocol.BotRef

/** 聊天这一层叠着的页；[ChatNavigation.chatPages] 从上往下排，返回逐层关掉。 */
internal sealed interface ChatPage {
    /** 认页、存滚动位置用，要能存进 Bundle；同一类页一个（换房间时原地换内容，和换页前一样）。 */
    val key: String

    data object Confirmations : ChatPage { override val key = "confirmations" }
    data object Backup : ChatPage { override val key = "backup" }
    data class Profile(val bot: BotRef) : ChatPage { override val key = "profile" }
    data object Starting : ChatPage { override val key = "starting" }
    data class Room(val id: String) : ChatPage { override val key = "room" }
    data object RoomList : ChatPage { override val key = "list" }
}

/**
 * 现在叠着的页，最上面的在前、聊天列表垫底。顺序就是显示的优先级：关掉最上面那页，露出来的是下一项。
 * 关房间要回别的页时房间下面不垫列表（返回由房间自己接，见 `ChatRoomPage`）。
 */
internal fun ChatNavigation.chatPages(): List<ChatPage> = buildList {
    if (showingConfirmations) add(ChatPage.Confirmations)
    if (showingBackup) add(ChatPage.Backup)
    botProfile?.let { add(ChatPage.Profile(it)) }
    if (startingChat) add(ChatPage.Starting)
    val room = openRoom
    if (room != null) {
        add(ChatPage.Room(room))
        if (returnsElsewhere) return@buildList
    }
    add(ChatPage.RoomList)
}

internal fun ChatNavigation.close(page: ChatPage) {
    when (page) {
        ChatPage.Confirmations -> showingConfirmations = false
        ChatPage.Backup -> showingBackup = false
        is ChatPage.Profile -> botProfile = null
        ChatPage.Starting -> startingChat = false
        is ChatPage.Room -> closeRoom()
        ChatPage.RoomList -> Unit
    }
}
