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

/**
 * 聊天页打开的房间。应用页和应用商店里点 Agent 插件时直接打开和它的 Bot 的私聊，关掉房间回到点的那一页。
 * [selectTab] 和房间一起换，不然中间会闪一下聊天列表。
 */
class ChatNavigation(
    openRoom: MutableState<String?>,
    returnTab: MutableState<Int?>,
    showingBackup: MutableState<Boolean>,
    startingChat: MutableState<Boolean>,
    private val selectTab: (Int) -> Unit,
) {
    var openRoom by openRoom
    private var returnTab by returnTab
    var showingBackup by showingBackup
    var startingChat by startingChat

    /** 刚发起了和这个用户的私聊，房间出现后打开。 */
    private var awaitedPeer by mutableStateOf<String?>(null)
    private var awaitedFrom = CHAT

    /** 要打开的房间，同步到了再打开：点通知冷启动时，会话还没恢复。 */
    private var awaitedRoom: String? = null

    /** 发起了私聊，房间还没出现。 */
    val isOpeningChat get() = awaitedPeer != null

    fun closeRoom() {
        openRoom = null
        returnTab?.let(selectTab)
        returnTab = null
    }

    /** 打开 [roomId]。这时还没有同步到它的话，等它出现。 */
    fun openWhenJoined(core: ClientCore, roomId: String) {
        awaitedPeer = null
        openRoom = null
        returnTab = null
        awaitedRoom = roomId
        onUpdate(core.chat())
    }

    /**
     * 从 [from] 这一页打开和 [userId] 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开，这期间留在原页；
     * 对方的邀请还没接受时到聊天列表，邀请就在那里。返回核心拒绝的原因。
     */
    fun openDirectChat(core: ClientCore, userId: String, from: Int = CHAT): String? {
        awaitedPeer = null
        awaitedRoom = null
        openRoom = null
        returnTab = null
        // 聊天页不在前台时没有收集，取核心当前的状态。
        val rooms = core.chat().rooms.filter { it.directUserId == userId }
        val joined = rooms.firstOrNull { it.membership == RoomMembership.JOINED }
        var rejection: String? = null
        if (joined != null) {
            show(joined.roomId, from)
        } else if (rooms.isEmpty()) {
            rejection = core.send(Action.StartDirectChat(userId))
            if (rejection == null) {
                awaitedPeer = userId
                awaitedFrom = from
            }
        } else {
            selectTab(CHAT)
        }
        return rejection
    }

    private fun show(roomId: String, from: Int) {
        returnTab = from.takeIf { it != CHAT }
        openRoom = roomId
        selectTab(CHAT)
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
            show(room.roomId, awaitedFrom)
        } else if (viewModel.problem != null) {
            // 发起失败的原因显示在聊天页。
            awaitedPeer = null
            selectTab(CHAT)
        }
    }
}

@Composable
fun rememberChatNavigation(selectTab: (Int) -> Unit): ChatNavigation {
    val openRoom = rememberSaveable { mutableStateOf<String?>(null) }
    val returnTab = rememberSaveable { mutableStateOf<Int?>(null) }
    val showingBackup = rememberSaveable { mutableStateOf(false) }
    val startingChat = rememberSaveable { mutableStateOf(false) }
    return remember { ChatNavigation(openRoom, returnTab, showingBackup, startingChat, selectTab) }
}
