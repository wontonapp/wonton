package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AskFailure
import app.wonton.protocol.AskState
import app.wonton.protocol.BotRef
import app.wonton.protocol.ChatAction
import app.wonton.protocol.ChatContact
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.DefaultBotVisibility
import app.wonton.protocol.OwnPrivateBotView
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.TabRef

/** 一次“问 Bot”的回执还没到时记着的请求；只认这个动作 ID 的回执。 */
private data class PendingAsk(val id: UInt, val text: String, val roomId: String?)

/** 问 Bot 失败的原因和重试要用的原文；关掉就收起。 */
data class AskFailureBanner(
    val text: String,
    val roomId: String?,
    val reason: AskFailure,
    val detail: String,
)

/// 旧核心没有回执时的等待：超过它仍没回执，就按旧行为打开默认 Agent。
private const val LEGACY_RECEIPT_GRACE_MS = 2_000L

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

    /** 问 Bot 的动作 ID、待处理回执和失败提示；只认自己发起的动作对应的回执。 */
    inner class AskNavigation {
        var askFailure by mutableStateOf<AskFailureBanner?>(null)
        private var pendingAsk: PendingAsk? = null
        private var askRequestId: UInt = 0u
        private val legacyHandler = Handler(Looper.getMainLooper())
        private var legacyOpen: Runnable? = null

        /** 当前开着的房间能不能原样当作“问 Bot”的目标：Bot 私聊、本人 Bot 的主房间，或者只读的通知房。 */
        fun currentBotRoomId(viewModel: ChatViewModel): String? {
            val id = openRoom
            val row = id?.let { room -> viewModel.rooms.firstOrNull { it.roomId == room } }
            val own = viewModel.ownPrivateBot
            return when {
                id == null || row == null -> null
                row.isReadOnly || row.botChat != null -> id
                own is OwnPrivateBotView.Ready && row.directUserId == own.userId -> id
                viewModel.bots.any { it.userId == row.directUserId } -> id
                else -> null
            }
        }

        /** 发起一次“问 Bot”；`roomId` 是当前房间，没有就 null，由核心按自己的顺序解析并写回执。 */
        fun startAsk(core: ClientCore, text: String, roomId: String?): Problem? {
            askFailure = null
            core.send(Action.Chat(ChatAction.DismissChatProblem))
            askRequestId += 1u
            val id = askRequestId
            pendingAsk = PendingAsk(id, text, roomId)
            val rejection = core.send(
                Action.Chat(ChatAction.AskAgent(text = text, roomId = roomId, requestId = id))
            )
            if (rejection != null) {
                pendingAsk = null
            } else {
                scheduleLegacyOpen(core, id)
            }
            return rejection
        }

        /// 旧核心不会写回执：等一会儿还没对上就按旧行为打开默认 Agent 的私聊。
        private fun scheduleLegacyOpen(core: ClientCore, id: UInt) {
            legacyOpen?.let(legacyHandler::removeCallbacks)
            val open = Runnable { legacyOpenDefaultAgent(core, id) }
            legacyOpen = open
            legacyHandler.postDelayed(open, LEGACY_RECEIPT_GRACE_MS)
        }

        private fun legacyOpenDefaultAgent(core: ClientCore, id: UInt) {
            if (pendingAsk?.id != id) return
            val onboarding = core.onboarding()
            val peer = onboarding.defaultBotUserId
            if (onboarding.defaultBotVisibility != DefaultBotVisibility.VISIBLE || peer.isNullOrEmpty()) {
                askFailure = AskFailureBanner(text = "", roomId = null, reason = AskFailure.BOT_NOT_READY, detail = "")
                return
            }
            if (pendingAsk?.text.isNullOrEmpty()) {
                core.send(Action.Chat(ChatAction.DismissChatProblem))
            }
            openDirectChat(core, peer, acceptInvite = true)
        }

        /** 重试上一次失败：沿用同一个房间，换一个新的动作 ID。 */
        fun retryAsk(core: ClientCore): Problem? {
            val failure = askFailure ?: return null
            askFailure = null
            val rejection = startAsk(core, failure.text, failure.roomId)
            if (rejection != null) askFailure = failure
            return rejection
        }

        /** 回执对上了就按状态开房间或留提示；Pending 不动。 */
        fun settleAsk(viewModel: ChatViewModel) {
            val pending = pendingAsk ?: return
            val receipt = viewModel.ask
            if (receipt == null || receipt.requestId != pending.id) return
            legacyOpen?.let(legacyHandler::removeCallbacks)
            legacyOpen = null
            pendingAsk = null
            when (receipt.state) {
                AskState.PENDING -> {}
                AskState.SENT -> {
                    val roomId = receipt.roomId
                    if (roomId != null) {
                        if (viewModel.rooms.any { it.roomId == roomId && it.membership == RoomMembership.JOINED }) {
                            show(roomId, TabRef.Chat)
                        } else {
                            awaitedPeer = null
                            awaitedRoom = roomId
                            awaitedSideChat = null
                            openRoom = null
                            returnTab = null
                            selectTab(TabRef.Chat)
                        }
                    }
                }
                AskState.FAILED -> askFailure = AskFailureBanner(
                    text = pending.text,
                    roomId = pending.roomId,
                    reason = receipt.problem?.reason ?: AskFailure.OTHER,
                    detail = receipt.problem?.detail ?: "",
                )
                AskState.OTHER -> {}
            }
        }
    }

    val ask = AskNavigation()

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
        ask.settleAsk(viewModel)
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
