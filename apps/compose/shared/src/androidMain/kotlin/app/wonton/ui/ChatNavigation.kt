package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.RoomMembership

/** 聊天页打开的房间。主屏上点 Agent 插件时直接打开和它的 Bot 的私聊。 */
class ChatNavigation(
    openRoom: MutableState<String?>,
    showingBackup: MutableState<Boolean>,
    startingChat: MutableState<Boolean>,
) {
    var openRoom by openRoom
    var showingBackup by showingBackup
    var startingChat by startingChat

    /** 刚发起了和这个用户的私聊，房间出现后打开。 */
    private var awaitedPeer: String? = null

    /** 要打开的房间，同步到了再打开：点通知冷启动时，会话还没恢复。 */
    private var awaitedRoom: String? = null

    /** 打开 [roomId]。这时还没有同步到它的话，等它出现。 */
    fun openWhenJoined(core: ClientCore, roomId: String) {
        awaitedPeer = null
        openRoom = null
        awaitedRoom = roomId
        onUpdate(core.chat())
    }

    /**
     * 打开和 [userId] 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开；对方的邀请还没接受时停在列表上，
     * 邀请就在那里。返回核心拒绝的原因。
     */
    fun openDirectChat(core: ClientCore, userId: String): String? {
        awaitedPeer = null
        awaitedRoom = null
        openRoom = null
        // 聊天页不在前台时没有收集，取核心当前的状态。
        val rooms = core.chat().rooms.filter { it.directUserId == userId }
        val joined = rooms.firstOrNull { it.membership == RoomMembership.JOINED }
        var rejection: String? = null
        if (joined != null) {
            openRoom = joined.roomId
        } else if (rooms.isEmpty()) {
            rejection = core.send(Action.StartDirectChat(userId))
            if (rejection == null) awaitedPeer = userId
        }
        return rejection
    }

    fun onUpdate(viewModel: ChatViewModel) {
        val awaited = awaitedRoom
        if (awaited != null && viewModel.rooms.any { it.roomId == awaited && it.membership == RoomMembership.JOINED }) {
            awaitedRoom = null
            openRoom = awaited
        }
        val peer = awaitedPeer ?: return
        val room = viewModel.rooms.firstOrNull { it.directUserId == peer && it.membership == RoomMembership.JOINED }
        if (room != null) {
            awaitedPeer = null
            openRoom = room.roomId
        }
    }
}

@Composable
fun rememberChatNavigation(): ChatNavigation {
    val openRoom = rememberSaveable { mutableStateOf<String?>(null) }
    val showingBackup = rememberSaveable { mutableStateOf(false) }
    val startingChat = rememberSaveable { mutableStateOf(false) }
    return remember { ChatNavigation(openRoom, showingBackup, startingChat) }
}
