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
import app.wonton.protocol.ChatAction
import app.wonton.protocol.BotRef
import app.wonton.protocol.ChatContact
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.TabRef

/**
 * 聊天页打开的房间。应用页和应用商店里点 Agent 插件时直接打开和它的 Bot 的私聊，关掉房间回到点的那一页。
 * [selectTab] 和房间一起换，不然中间会闪一下聊天列表。
 */
class ChatNavigation(
    private val openRoomState: MutableState<String?>,
    sideChat: MutableState<String?>,
    returnTab: MutableState<TabRef?>,
    showingBackup: MutableState<Boolean>,
    startingChat: MutableState<Boolean>,
    private val selectTab: (TabRef) -> Unit,
) {
    /** 开着的房间。换房间时旁聊和这个房间的一次性提示一起清掉。 */
    var openRoom: String?
        get() = openRoomState.value
        set(value) {
            if (value != openRoomState.value) {
                sideChat = null
                composingSideChat = false
                noSideChatsRoom = null
            }
            openRoomState.value = value
        }

    /** 开着的房间里开着的旁聊（根的事件 ID，ADR-0093）；为空是主对话。主对话在它下面照常订阅着。 */
    var sideChat by sideChat

    /** 进房间就弹开新旁聊的面板：“新建聊天”选了支持旁聊的 AI（ADR-0093 §4）。 */
    var composingSideChat by mutableStateOf(false)

    /** “新建聊天”选的 AI 不支持旁聊、打开的是主对话：这个房间里提示一句（ADR-0093 §7）。 */
    var noSideChatsRoom by mutableStateOf<String?>(null)
    private var returnTab by returnTab
    var showingBackup by showingBackup
    var startingChat by startingChat
    var botProfile by mutableStateOf<BotRef?>(null)
    var showingConfirmations by mutableStateOf(false)

    fun openConfirmations() {
        reset()
        showingConfirmations = true
        selectTab(TabRef.Chat)
    }

    /** 主人在交接页点了“稍后”（ADR-0087）：只在这台 App 里先收起，聊天列表顶上的提醒再打开。 */
    var keyHandoffDeferred by mutableStateOf(false)

    /** 刚发起了和这个用户的私聊，房间出现后打开。 */
    private var awaitedPeer by mutableStateOf<String?>(null)
    private var awaitedFrom: TabRef = TabRef.Chat

    /** 要打开的房间，同步到了再打开：点通知冷启动时，会话还没恢复。 */
    private var awaitedRoom: String? = null

    /** 和 [awaitedRoom] 一起：通知来自旁聊时，房间打开后进这个旁聊。 */
    private var awaitedSideChat: String? = null

    /** 发起了私聊，房间还没出现。 */
    val isOpeningChat get() = awaitedPeer != null

    fun closeRoom() {
        botProfile = null
        openRoom = null
        returnTab?.let(selectTab)
        returnTab = null
    }

    /** 外部导航清掉旧的返回页和异步等待，不能在新页面打开后被旧房间覆盖。 */
    fun reset() {
        botProfile = null
        awaitedPeer = null
        awaitedRoom = null
        awaitedSideChat = null
        openRoom = null
        returnTab = null
        showingBackup = false
        startingChat = false
        showingConfirmations = false
    }

    /** 打开 [roomId]，[sideChatRoot] 不为空时进这个旁聊。这时还没有同步到它的话，等它出现。 */
    fun openWhenJoined(core: ClientCore, roomId: String, sideChatRoot: String? = null) {
        botProfile = null
        awaitedPeer = null
        openRoom = null
        returnTab = null
        awaitedRoom = roomId
        awaitedSideChat = sideChatRoot
        onUpdate(core.chat())
        selectTab(TabRef.Chat)
    }

    /** 进 [roomId] 里的旁聊 [root]；房间已经开着时只换上旁聊，主对话不重新订阅。 */
    fun openSideChat(roomId: String, root: String) {
        openRoom = roomId
        sideChat = root
        composingSideChat = false
    }

    /**
     * “新建聊天”选了 [contact]（ADR-0093 §4）：有能开旁聊的私聊就进去弹开新旁聊的面板；已经有和这个 AI 的私聊、
     * 但它不支持旁聊，打开主对话并提示一句；其余照旧发起或打开私聊。[isBot] 判断已有私聊的对方是不是 AI。
     */
    fun startChatWith(core: ClientCore, contact: ChatContact, isBot: (RoomRow) -> Boolean): Problem? {
        val sideChatRoom = contact.sideChatRoomId
        if (sideChatRoom != null) {
            awaitedPeer = null
            awaitedRoom = null
            show(sideChatRoom, TabRef.Chat)
            composingSideChat = true
            return null
        }
        val rejection = openDirectChat(core, contact.userId)
        val room = openRoom
        val row = core.chat().rooms.firstOrNull { it.roomId == room }
        if (rejection == null && row != null && isBot(row)) noSideChatsRoom = room
        return rejection
    }

    /**
     * 从 [from] 这一页打开和 [peer] 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开，这期间留在原页；
     * 邀请默认留在列表由主人接受；[acceptInvite] 用于主人主动打开默认 Agent 的系统入口。
     */
    fun openDirectChat(
        core: ClientCore, peer: String, from: TabRef = TabRef.Chat, acceptInvite: Boolean = false,
    ): Problem? {
        awaitedPeer = null
        awaitedRoom = null
        openRoom = null
        returnTab = null
        val userId = try {
            core.resolveUserId(peer)
        } catch (rejection: CoreException) {
            return rejection.problem
        }
        // 聊天页不在前台时没有收集，取核心当前的状态。
        val rooms = core.chat().rooms.filter { it.directUserId == userId }
        val joined = rooms.firstOrNull { it.membership == RoomMembership.JOINED }
        val invited = rooms.firstOrNull { it.membership == RoomMembership.INVITED }
        var rejection: Problem? = null
        if (joined != null) {
            show(joined.roomId, from)
        } else if (acceptInvite && invited != null) {
            rejection = core.send(Action.Chat(ChatAction.JoinRoom(invited.roomId)))
            if (rejection == null) {
                awaitedPeer = userId
                awaitedFrom = from
            }
        } else if (rooms.isEmpty()) {
            rejection = core.send(Action.Chat(ChatAction.StartDirectChat(userId)))
            if (rejection == null) {
                awaitedPeer = userId
                awaitedFrom = from
            }
        } else {
            selectTab(TabRef.Chat)
        }
        return rejection
    }

    private fun show(roomId: String, from: TabRef) {
        botProfile = null
        returnTab = from.takeIf { it != TabRef.Chat }
        openRoom = roomId
        selectTab(TabRef.Chat)
    }

    fun onUpdate(viewModel: ChatViewModel) {
        val awaited = awaitedRoom
        if (awaited != null && viewModel.rooms.any { it.roomId == awaited && it.membership == RoomMembership.JOINED }) {
            awaitedRoom = null
            openRoom = awaited
            sideChat = awaitedSideChat
            awaitedSideChat = null
        }
        val peer = awaitedPeer ?: return
        val room = viewModel.rooms.firstOrNull { it.directUserId == peer && it.membership == RoomMembership.JOINED }
        if (room != null) {
            awaitedPeer = null
            show(room.roomId, awaitedFrom)
        } else if (viewModel.problem != null) {
            // 发起失败的原因显示在聊天页。
            awaitedPeer = null
            selectTab(TabRef.Chat)
        }
    }
}

@Composable
fun rememberChatNavigation(selectTab: (TabRef) -> Unit): ChatNavigation {
    val openRoom = rememberSaveable { mutableStateOf<String?>(null) }
    val sideChat = rememberSaveable { mutableStateOf<String?>(null) }
    val returnTab = remember { mutableStateOf<TabRef?>(null) }
    val showingBackup = rememberSaveable { mutableStateOf(false) }
    val startingChat = rememberSaveable { mutableStateOf(false) }
    return remember { ChatNavigation(openRoom, sideChat, returnTab, showingBackup, startingChat, selectTab) }
}
