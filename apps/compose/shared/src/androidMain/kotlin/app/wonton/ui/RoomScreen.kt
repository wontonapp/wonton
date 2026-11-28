package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.client.ClientCore
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.BotView
import app.wonton.protocol.BotAccess
import app.wonton.protocol.BotRef
import app.wonton.protocol.ComposerEdit
import app.wonton.protocol.ComposerViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.JumpState
import app.wonton.protocol.KeyNotice
import app.wonton.protocol.MessageContent
import app.wonton.protocol.RoomCommand
import app.wonton.protocol.RoomPlugin
import app.wonton.protocol.RoomRow
import app.wonton.protocol.RoomWidget
import app.wonton.protocol.SideChatCard
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineDiff
import app.wonton.protocol.TimelineFocus
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.protocol.TimelineViewport
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonBarStatus
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Home
import app.wonton.ui.icons.LockKey
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.filter

private const val TAG = "RoomScreen"

internal class TimelineState {
    val items = mutableStateListOf<TimelineItem>()
    val list = LazyListState()
    var backPagination by mutableStateOf(BackPagination.IDLE)

    /** 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。 */
    var loaded by mutableStateOf(false)
    var failure by mutableStateOf<Problem?>(null)

    /** 核心算的位置：在不在底部、离开底部时的新消息数（ADR-0071）。 */
    var viewport by mutableStateOf<TimelineViewport>(TimelineViewport.Bottom)

    /** 第一批的落点：进房间停在这一条上（ADR-0071）。 */
    var anchor by mutableStateOf<String?>(null)

    /** 有消息解不开时给主人的入口，核心按加密状态给。 */
    var notice by mutableStateOf<KeyNotice?>(null)

    /** 时间线被清空重建的次数：重建的 Reset 里带新落点，壳要重新摆。 */
    var clears by mutableIntStateOf(0)

    /** 每一批更新的序号，编辑流和内容增高都要触发布局跟随。 */
    var revision by mutableIntStateOf(0)
    /** 核心要求自己发出的消息回到底部的次数。 */
    var bottomRequest by mutableIntStateOf(0)
    var handledBottomRequest by mutableIntStateOf(0)
    /** 打不开时点“重试”加一，按它重新订阅。 */
    var attempt by mutableIntStateOf(0)

    /** 旁聊页顶上的标题和状态（ADR-0093）；主对话为空。 */
    var sideChat by mutableStateOf<SideChatCard?>(null)
    /** 顶栏的 widget 入口和房间里有没有人在通话（ADR-0096）。 */
    var call by mutableStateOf(RoomCall())
    var confirmations by mutableStateOf<List<ConfirmationView>>(emptyList())
    var openedConfirmation by mutableStateOf<String?>(null)

    /** 输入框正在编辑哪条（ADR-0109 第 10 节），每一批都带当前值。 */
    var editing by mutableStateOf<ComposerEdit?>(null)

    /** 群里在场的 AI（本账号认识的 Bot 的 Matrix 用户 ID）；私聊不算。null 是成员名单还没取全、
     * 在场的算不准，别下“群里没有 AI”的结论；空列表才是确定没有。 */
    var roomBots by mutableStateOf<List<String>?>(null)

    /** 正滚到哪一条（跳转、跳回去的落点）：只在要滚的那一批里有。 */
    var focus by mutableStateOf<TimelineFocus?>(null)
    /** 跳转的进展（ADR-0109）：加载中原消息时在引用上转圈，找不到时轻提示。 */
    var jump by mutableStateOf(JumpState.IDLE)
    /** 正在跳的那条引用（原消息的事件 ID）；只在加载中非空，落地或失败就收掉。 */
    var jumpQuote by mutableStateOf<String?>(null)
    /** 壳报给核心的跳转序号，每发起一次加一。 */
    var jumpSeq by mutableIntStateOf(0)
    /** 核心收下的跳转序号：同一条引用连点两下，落点一样也要重新滚。 */
    var focusRevision by mutableIntStateOf(0)
    /** 跳之前那一条（栈顶）：留着时“向下”按钮先退回到它。 */
    var backTo by mutableStateOf<String?>(null)
    /** 正在输入的其他人（显示名），顶栏状态行用。 */
    var typing by mutableStateOf<List<String>>(emptyList())
    /** 房间里各 Bot 的斜杠命令（ADR-0054）。 */
    var commands by mutableStateOf<List<RoomCommand>>(emptyList())
    /** 命令表被截掉的条数。 */
    var commandsOmitted by mutableIntStateOf(0)

    fun apply(diff: TimelineDiff) {
        when (diff) {
            is TimelineDiff.Append -> items.addAll(diff.items)
            TimelineDiff.Clear -> {
                items.clear()
                clears++
            }
            is TimelineDiff.PushFront -> items.add(0, diff.item)
            is TimelineDiff.PushBack -> items.add(diff.item)
            TimelineDiff.PopFront -> items.removeAt(0)
            TimelineDiff.PopBack -> items.removeAt(items.lastIndex)
            is TimelineDiff.Insert -> items.add(diff.index.toInt(), diff.item)
            is TimelineDiff.Set -> items[diff.index.toInt()] = diff.item
            is TimelineDiff.Remove -> items.removeAt(diff.index.toInt())
            is TimelineDiff.Truncate -> items.removeRange(diff.length.toInt(), items.size)
            is TimelineDiff.Reset -> {
                items.clear()
                items.addAll(diff.items)
                clears++
            }
        }
    }
}

/**
 * 一条时间线的聊天页：[target] 没有旁聊根时是房间的主对话，有时是那个旁聊（ADR-0093）。[sideChats] 是和旁聊有关的入口。
 */
@Composable
internal fun RoomRoute(
    core: ClientCore,
    target: TimelineTarget,
    sideChats: SideChatEntry,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
    botAccess: BotAccess?,
    bots: List<BotView>,
    plugins: PluginsState,
    rooms: List<RoomRow>,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenBackup: () -> Unit,
    onOpenBotProfile: (BotRef) -> Unit,
    onClose: () -> Unit,
) {
    val timeline = remember(target) { TimelineState() }
    LaunchedEffect(core, target, timeline.attempt) {
        timeline.failure = null
        try {
            core.timelineUpdates(target).collect { update ->
                update.diffs.forEach(timeline::apply)
                timeline.backPagination = update.backPagination
                timeline.viewport = update.viewport
                timeline.notice = update.keyNotice
                update.anchor?.let { timeline.anchor = it }
                timeline.sideChat = update.sideChat
                timeline.call = RoomCall(update.widgets, update.callActive)
                timeline.confirmations = update.confirmations
                timeline.roomBots = update.roomBots
                timeline.focus = update.focus
                timeline.jump = update.jump
                // 找不到就收转圈；落地由列表滚完那一下收，滚之前第二下仍算重复点。
                if (update.jump == JumpState.UNAVAILABLE) timeline.jumpQuote = null
                timeline.focusRevision = update.focusRevision.toInt()
                timeline.backTo = update.backTo
                timeline.editing = update.editing
                timeline.typing = update.typing
                timeline.commands = update.commands
                timeline.commandsOmitted = update.commandsOmitted.toInt()
                if (update.scrollToBottom) timeline.bottomRequest++
                timeline.revision++
                timeline.loaded = true
            }
            currentCoroutineContext().ensureActive()
            onClose()
        } catch (failure: CoreException) {
            timeline.failure = failure.problem
        }
    }
    // 附件区和点了发送、还没交出去的消息（ADR-0094）：附件区画在输入框上方，待发消息画在最新一条下面。
    // 只有主对话有附件区，旁聊里字和照片直接发（ADR-0093）。
    var composer by remember(target) { mutableStateOf<ComposerViewModel?>(null) }
    if (target.threadRoot == null) {
        LaunchedEffect(core, target) {
            try {
                core.composerUpdates(target.roomId).collect { composer = it }
            } catch (failure: CoreException) {
                onReject(failure.problem)
            }
        }
    }
    val first = remember(core) { core.transfers() }
    val transfers by produceState(first, core) { core.transfersUpdates().collect { value = it } }
    CompositionLocalProvider(
        LocalFiles provides FilesContext(target.roomId, transfers, onAction, onReject),
    ) {
        RoomScreen(
            core,
            target,
            sideChats,
            title,
            isDirect,
            isReadOnly,
            bot,
            botAccess,
            bots,
            plugins,
            rooms,
            timeline,
            composer,
            onAction,
            onReject,
            onOpenRoom,
            onOpenBackup,
            onOpenBotProfile,
            onClose,
        )
    }
}

/** 长按的是哪条消息，它的气泡在屏幕上的位置。 */
private class MessageMenuTarget(val itemId: String, val message: TimelineMessage, val bubble: Rect)

@Composable
private fun RoomScreen(
    core: ClientCore,
    target: TimelineTarget,
    sideChats: SideChatEntry,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
    botAccess: BotAccess?,
    bots: List<BotView>,
    plugins: PluginsState,
    rooms: List<RoomRow>,
    timeline: TimelineState,
    composer: ComposerViewModel?,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenBackup: () -> Unit,
    onOpenBotProfile: (BotRef) -> Unit,
    onClose: () -> Unit,
) {
    var menu by remember { mutableStateOf<MessageMenuTarget?>(null) }
    val openMenu = { item: TimelineItem, message: TimelineMessage, bounds: Rect ->
        menu = MessageMenuTarget(item.id, message, bounds)
    }
    val botNotice = botAccessNotice(botAccess)
    // 资格未知（离线、核心还没给）照常能发，进离线队列；只有确定的不能用才禁用。
    val botCanSend = botAccess !is BotAccess.Unusable
    val visible = messageMenuBounds()
    val roomId = target.roomId
    val draft = rememberRoomDraft(core, target, onAction)
    // 转发、撤回都要时间线条目的 id，菜单开着的那条。
    var forwarding by remember { mutableStateOf<String?>(null) }
    var redacting by remember { mutableStateOf<MessageMenuTarget?>(null) }
    TypingStatus(isDirect, timeline.typing) {
        Box(Modifier.fillMaxSize()) {
            // 菜单开着时背后的页面模糊（ADR-0029）。Android 12 以下没有模糊，只靠菜单的压暗。
            Box(if (menu != null) Modifier.blur(WontonTheme.size.menuBlur) else Modifier) {
                RoomSurface(
                    bar = {
                        RoomBar(
                            target, sideChats, title, isDirect, bot, plugins, timeline.sideChat, onClose,
                            onOpenBotProfile = { onOpenBotProfile(it) },
                            call = timeline.call,
                            onOpenWidget = { onAction(Action.Pages(PageAction.OpenPage(it.tab))) },
                        )
                    },
                ) { backdrop ->
                    Conversation(
                        core, target, sideChats, isDirect, isReadOnly, timeline, composer, draft, onAction, onReject,
                        rooms.firstOrNull { it.roomId == roomId }?.plugin, title, openMenu,
                        onOpenBackup, backdrop,
                        // 群里能用的 AI：本账号认识的 Bot 里在场的；私聊为空，不显示。成员没取全（null）照传，
                        // 让 RoomAiLine 不下“群里没有”的结论。
                        roomAi = if (isDirect) emptyList() else timeline.roomBots?.mapNotNull { id ->
                            bots.firstOrNull { it.userId == id }
                        },
                        botNotice = botNotice,
                        botCanSend = botCanSend,
                        plugins = plugins,
                        onOpenBotProfile = onOpenBotProfile,
                    )
                }
            }
            RoomOverlays(
                menu, forward = forwarding, redacting = redacting, target = target, rooms = rooms, bots = bots,
                plugins = plugins, timeline = timeline, draft = draft, visible = visible, core = core,
                onAction = onAction, onOpenRoom = onOpenRoom,
                onMenuDismiss = { menu = null },
                onForward = { forwarding = it },
                onRedact = { redacting = it },
            )
            if (sideChats.composing) {
                NewSideChatSheet(
                    core,
                    target.roomId,
                    onStarted = { root ->
                        sideChats.onComposingChange(false)
                        sideChats.onOpen(root)
                    },
                    onDismiss = { sideChats.onComposingChange(false) },
                )
            }
        }
    }
}

/**
 * 有人正在输入时把顶栏副标题换成它（ADR-0109）：私聊不点名，群里点名和人数。连不上的长提示各自占一行，不抢。
 */
@Composable
private fun TypingStatus(isDirect: Boolean, typing: List<String>, content: @Composable () -> Unit) {
    val line = when {
        typing.isEmpty() -> null
        isDirect -> stringResource(R.string.timeline_typing)
        typing.size == 1 -> stringResource(R.string.timeline_typing_group, typing.first())
        else -> stringResource(R.string.timeline_typing_more, typing.first(), typing.size - 1)
    }
    val status = LocalWontonBarStatus.current
    val shown = line?.let { WontonBarStatus(status.title, it, status.line) } ?: status
    CompositionLocalProvider(LocalWontonBarStatus provides shown) { content() }
}

/** 压在聊天页上的浮层：长按菜单、转发、撤回确认、确认卡片。 */
@Composable
private fun RoomOverlays(
    menu: MessageMenuTarget?,
    forward: String?,
    redacting: MessageMenuTarget?,
    target: TimelineTarget,
    rooms: List<RoomRow>,
    bots: List<BotView>,
    plugins: PluginsState,
    timeline: TimelineState,
    draft: RoomDraft,
    visible: Rect,
    core: ClientCore,
    onAction: (Action) -> Unit,
    onOpenRoom: (String) -> Unit,
    onMenuDismiss: () -> Unit,
    onForward: (String?) -> Unit,
    onRedact: (MessageMenuTarget?) -> Unit,
) {
    menu?.let { opened ->
        val groups = remember(timeline.revision) { timeline.items.imageGroups() }
        MessageMenu(
            message = opened.message,
            bubble = opened.bubble,
            visible = visible,
            core = core,
            groupImages = opened.message.group?.let { groups[opened.message.senderId to it.id] },
            onReply = draft::replyTo,
            onEdit = { text ->
                onAction(Action.Timeline(TimelineAction.SetTyping(target, false)))
                draft.startEditing(text)
                onAction(Action.Timeline(TimelineAction.StartEdit(target, opened.itemId)))
            },
            onForward = { onForward(opened.itemId) },
            onRedact = { onRedact(opened) },
            onDismiss = onMenuDismiss,
        )
    }
    forward?.let { itemId ->
        ForwardSheet(
            rooms = rooms.filter { it.forwardTarget(target.roomId) },
            bots = bots,
            plugins = plugins,
            // 核心一次只发一个房间：挨个带上附言派发；只选了一个才跟过去，选几个留在原房间。
            onSend = { rows, comment ->
                // 壳先 trim 判空白，只含空白当没有；核心还会再 trim 一遍。
                val note = comment.trim().ifEmpty { null }
                rows.forEach { row ->
                    onAction(Action.Timeline(TimelineAction.ForwardMessage(target, itemId, row.roomId, note)))
                }
                if (rows.size == 1) onOpenRoom(rows.first().roomId)
            },
            onDismiss = { onForward(null) },
        )
    }
    redacting?.let { opened ->
        RedactConfirm(
            opened.message,
            onConfirm = {
                onAction(Action.Timeline(TimelineAction.RedactMessage(target, opened.itemId)))
                onRedact(null)
            },
            onDismiss = { onRedact(null) },
        )
    }
    RoomConfirmationSheet(
        timeline.confirmations,
        timeline.openedConfirmation,
        onDismiss = { timeline.openedConfirmation = null },
        onAction = onAction,
    )
}

/**
 * 顶栏。和 Bot 的私聊显示 Bot 的状态头像和名字（ADR-0044），名字下面常空着，有人正在输入时写那一行（ADR-0109），
 * 群聊的标题居中；支持旁聊的房间带开旁聊的按钮。私聊点名字或头像进 Bot 资料页。
 * 旁聊页的标题是旁聊的标题（ADR-0093），状态在顶栏下面一行。
 * 房间里装着 widget（通话）时带它的入口：点了全屏打开 widget 页，关掉回到这个房间（ADR-0096）。
 */
@Composable
private fun RoomBar(
    target: TimelineTarget,
    sideChats: SideChatEntry,
    title: String,
    isDirect: Boolean,
    bot: BotView?,
    plugins: PluginsState,
    header: SideChatCard?,
    onClose: () -> Unit,
    onOpenBotProfile: (BotRef) -> Unit,
    call: RoomCall,
    onOpenWidget: (RoomWidget) -> Unit,
) {
    if (target.threadRoot != null) {
        WontonPushedBar(
            header?.title?.takeIf { it.isNotBlank() } ?: stringResource(R.string.side_chat),
            onClose,
            showsStatus = true,
        )
        return
    }
    val barTitle = if (isDirect) bot?.name ?: title else title
    val profileBot = bot?.takeIf { isDirect }
    val barAvatar: (@Composable () -> Unit)? = profileBot?.let {
        { BotAvatar(it, plugins, WontonTheme.size.avatar) }
    }
    WontonPushedBar(
        barTitle,
        onClose,
        showsStatus = true,
        showsStatusLine = true,
        avatar = barAvatar,
        avatarClick = profileBot?.let { open -> { onOpenBotProfile(open.bot) } },
        avatarLabel = stringResource(R.string.bot_profile_open),
        centeredTitle = !isDirect,
        actions = {
            RoomCallEntry(call, onOpenWidget)
            if (sideChats.supported) {
                WontonIconButton(
                    WontonIcon.Compose,
                    stringResource(R.string.side_chat_new),
                    { sideChats.onComposingChange(true) },
                )
            }
        },
    )
}

/** 消息列表、传输条和输入栏。打开失败时列表位置换成原因和重试。 */
@Composable
private fun Conversation(
    core: ClientCore,
    target: TimelineTarget,
    sideChats: SideChatEntry,
    isDirect: Boolean,
    isReadOnly: Boolean,
    timeline: TimelineState,
    composer: ComposerViewModel?,
    draft: RoomDraft,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    plugin: RoomPlugin?,
    title: String,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onOpenBackup: () -> Unit,
    backdrop: RoomBackdrop,
    roomAi: List<BotView>?,
    plugins: PluginsState,
    botNotice: Int?,
    botCanSend: Boolean,
    onOpenBotProfile: (BotRef) -> Unit,
) {
    val density = LocalDensity.current
    var composerHeight by remember { mutableIntStateOf(0) }
    var headerHeight by remember { mutableIntStateOf(0) }
    val barInset = LocalWontonTopInset.current
    val bottomInsets = WindowInsets.ime.union(WindowInsets.navigationBars)
    // 命令列表开没开：输入区的菜单键改它，翻时间线先收起来，好把列表让出去；“向下”钮开着时不画。
    var commandMenuShown by remember { mutableStateOf(false) }
    BoxWithConstraints(Modifier.fillMaxSize().windowInsetsPadding(bottomInsets)) {
        Column(Modifier.fillMaxSize()) {
            // 状态行自己占一行，占多高时间线就让开多高，不浮在气泡上；气泡只从它的下沿下面消失。
            Column(
                Modifier.fillMaxWidth().roomBackdrop(backdrop, top = true).padding(top = barInset)
                    .onSizeChanged { headerHeight = it.height }
                    .testTag("room-header"),
            ) {
                AgentPluginStatusBanner(plugin, title, plugins.viewModel, onAction)
                timeline.sideChat?.let { SideChatHeaderLine(it.state) }
                if (sideChats.unsupportedHint) NoSideChatsHint()
                // 群里能用的 AI；私聊不显示，Bot 本来就在顶栏（ADR-0044）。
                if (!isDirect && target.threadRoot == null) {
                    RoomAiLine(roomAi, plugins, onOpenBotProfile)
                }
            }
            Box(Modifier.weight(1f).fillMaxWidth()) {
                CompositionLocalProvider(
                    LocalWontonBottomInset provides with(density) { composerHeight.toDp() },
                    // 时间线从状态行下面开始，只有顶栏还压在上面。
                    LocalWontonTopInset provides barInset,
                ) {
                    val failure = timeline.failure
                    if (failure != null) {
                        Box(Modifier.fillMaxSize().padding(top = barInset)) {
                            ProblemView(failure) { timeline.attempt++ }
                        }
                    } else {
                        TimelineList(
                            core, target, sideChats, timeline, composer?.outgoing.orEmpty(),
                            showsSender = !isDirect,
                            onAction = onAction,
                            onOpenMenu = onOpenMenu,
                            onReply = { message -> message.replyAction?.let { draft.replyTo(it.message) } },
                            modifier = Modifier.fillMaxSize().recordRoomBackdrop(backdrop),
                            menuOpen = commandMenuShown,
                            onScroll = { commandMenuShown = false },
                        )
                    }
                }
            }
        }
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .roomBackdrop(backdrop, top = false)
                .testTag("composer-block")
                .onSizeChanged { composerHeight = it.height }
                // 键盘、确认卡片一起把输入区顶高时别高过剩下的地方，也别盖住顶栏，超出让上面几行先缩。
                .heightIn(max = maxHeight - with(density) { headerHeight.toDp() }),
        ) {
            // 输入框以上的几行先缩、在里头滚，输入框本身始终露着；块本身只到内容高，不留空档。
            Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState())) {
                timeline.notice?.let { KeyNoticeRow(it, onOpenBackup) }
                TransfersStrip(target.roomId, core)
                RoomConfirmationRow(timeline.confirmations) { timeline.openedConfirmation = it }
                botNotice?.let { BotChatNoticeRow(it) }
            }
            if (!isReadOnly) {
                Composer(
                    target = target,
                    draft = draft.text,
                    reply = draft.reply,
                    edit = timeline.editing,
                    focusToken = draft.focusToken,
                    composer = composer,
                    core = core,
                    onDraftChange = { text ->
                        draft.change(text)
                        onAction(Action.Timeline(TimelineAction.SetTyping(target, text.isNotEmpty())))
                    },
                    onSent = draft::sent,
                    onCommandSent = draft::commandSent,
                    menuOpen = commandMenuShown,
                    onMenuChange = { commandMenuShown = it },
                    onReplyCancel = draft::stopReplying,
                    onQuoteJump = { timeline.jumpTo(it, timeline.bottomMostItemId(), target, onAction) },
                    onEditCancel = {
                        onAction(Action.Timeline(TimelineAction.SetTyping(target, false)))
                        onAction(Action.Timeline(TimelineAction.CancelEdit(target)))
                    },
                    onRestoreDraft = draft::restore,
                    onTyping = { typing ->
                        onAction(Action.Timeline(TimelineAction.SetTyping(target, typing)))
                    },
                    onReject = onReject,
                    onAction = onAction,
                    commands = timeline.commands,
                    commandsOmitted = timeline.commandsOmitted,
                    // 草稿整句就是一条完整命令（没有多写的正文）时发核心给的整句，不自己拼。
                    enabled = botCanSend,
                    disabledProblem = botAccessBlockedProblem(botCanSend, botNotice),
                    dropCommand = timeline.commands.firstOrNull { it.text == draft.text.trim() }?.text,
                )
            }
        }
    }
}

/**
 * 有消息解不开时的一行说明和入口：账号设过备份就输恢复密钥，没设过就请另一台设备确认，整行点去备份页（ADR-0046）。
 * 这行常驻输入框上方，收成一行，别把能看到的消息挤走；更新的核心给出的类别没有按钮，说明照常显示。
 */
@Composable
private fun KeyNoticeRow(notice: KeyNotice, onOpenBackup: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .clickable(enabled = notice != KeyNotice.OTHER, onClick = onOpenBackup, role = Role.Button)
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.pageInset),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(
            WontonIcon.LockKey,
            // 装饰：右边的字说了要做什么。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconTiny),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            stringResource(notice.text),
            Modifier.weight(1f).padding(vertical = WontonTheme.spacing.s4),
            style = WontonTheme.typography.body,
            color = colors.textPrimary.toColor(),
        )
    }
}

/** 按类别说明密钥怎么找回；两条路的入口都在备份页（ADR-0046）。 */
private val KeyNotice.text: Int
    get() = when (this) {
        KeyNotice.ENTER_RECOVERY_KEY -> R.string.timeline_key_notice_recovery
        KeyNotice.VERIFY_WITH_OTHER_DEVICE -> R.string.timeline_key_notice_verify
        KeyNotice.OTHER -> R.string.timeline_undecryptable_notice
    }

/** 撤回对所有人生效，发出去之前先确认。 */
@Composable
private fun RedactConfirm(message: TimelineMessage, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    WontonDialog(
        title = if (message.isOwn) {
            stringResource(R.string.message_delete_everyone_title)
        } else {
            stringResource(R.string.message_delete_others_title, message.senderName)
        },
        message = stringResource(R.string.message_delete_everyone_message),
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(
                stringResource(R.string.message_delete_everyone_confirm),
                WontonButtonKind.DANGER,
                onConfirm,
            ),
        ),
    )
}

/// 这条消息带着的确认卡片 ID：Agent 的等待行靠它滚到卡片。
private val TimelineItem.cardId
    get() = ((content as? TimelineContent.Message)?.message?.body as? MessageContent.Confirmation)?.view?.card?.id

internal val TimelineItem.isOwn
    get() = (content as? TimelineContent.Message)?.message?.isOwn == true
