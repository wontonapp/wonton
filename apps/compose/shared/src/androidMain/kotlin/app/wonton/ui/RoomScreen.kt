package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.BotView
import app.wonton.protocol.BotRef
import app.wonton.protocol.ComposerViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.KeyNotice
import app.wonton.protocol.MessageContent
import app.wonton.protocol.OutgoingMessage
import app.wonton.protocol.PanelItem
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.SideChatCard
import app.wonton.protocol.TabRef
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineDiff
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.protocol.TimelineViewport
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.LockKey
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.filter

private const val TAG = "RoomScreen"

internal class TimelineState {
    val items = mutableStateListOf<TimelineItem>()
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

    /** 这个房间的面板（ADR-0052）：核心每一批带上。 */
    var panel by mutableStateOf<List<PanelItem>>(emptyList())
    var panelSelected by mutableStateOf<TabRef?>(null)
    /** 另有东西盖着消息列表（长按菜单、转发、删除确认），上推拉面板的入口不响应。 */
    var entryBlocked by mutableStateOf(false)

    /** 旁聊页顶上的标题和状态（ADR-0093）；主对话为空。 */
    var sideChat by mutableStateOf<SideChatCard?>(null)
    var confirmations by mutableStateOf<List<ConfirmationView>>(emptyList())
    var openedConfirmation by mutableStateOf<String?>(null)

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
                timeline.panel = update.panel
                timeline.panelSelected = update.panelSelected
                timeline.entryBlocked = update.entryBlocked
                timeline.sideChat = update.sideChat
                timeline.confirmations = update.confirmations
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
    // 旁聊页没有房间的面板（ADR-0052）：面板跟着主对话。
    val panel = if (target.threadRoot == null) LocalChatPanel.current else null
    CompositionLocalProvider(
        LocalFiles provides FilesContext(transfers, onAction, onReject),
        LocalChatPanel provides panel,
    ) {
        RoomScreen(
            core,
            target,
            sideChats,
            title,
            isDirect,
            isReadOnly,
            bot,
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

/**
 * 菜单能画在哪儿：窗口减去系统栏和键盘。弹层是另一个窗口，在里面拿不到这些，在这里算好传进去。
 * 横屏时导航栏或挖孔在侧边，左右也要让。
 */
@Composable
private fun messageMenuBounds(): Rect {
    val windowSize = LocalWindowInfo.current.containerSize
    val density = LocalDensity.current
    val safe = WindowInsets.safeDrawing
    val direction = LocalLayoutDirection.current
    return Rect(
        safe.getLeft(density, direction).toFloat(),
        safe.getTop(density).toFloat(),
        (windowSize.width - safe.getRight(density, direction)).toFloat(),
        (windowSize.height - safe.getBottom(density)).toFloat(),
    )
}

@Composable
private fun messageMenuBounds(): Rect {
    // 菜单能画在哪儿：窗口减去系统栏和键盘。弹层是另一个窗口，在里面拿不到这些，在这里算好传进去。
    // 横屏时导航栏或挖孔在侧边，左右也要让。
    val windowSize = LocalWindowInfo.current.containerSize
    val density = LocalDensity.current
    val safe = WindowInsets.safeDrawing
    val direction = LocalLayoutDirection.current
    return Rect(
        safe.getLeft(density, direction).toFloat(),
        safe.getTop(density).toFloat(),
        (windowSize.width - safe.getRight(density, direction)).toFloat(),
        (windowSize.height - safe.getBottom(density)).toFloat(),
    )
}

@Composable
private fun RoomScreen(
    core: ClientCore,
    target: TimelineTarget,
    sideChats: SideChatEntry,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
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
    val visible = messageMenuBounds()
    val roomId = target.roomId
    val draft = rememberRoomDraft(core, target, onAction)
    val panel = LocalChatPanel.current
    // 在组合里读一遍，面板数据变了才会重组，下面的 SideEffect 才会跟着跑。
    val panelPages = timeline.panel
    val panelSelected = timeline.panelSelected
    val entryBlocked = timeline.entryBlocked
    SideEffect {
        panel?.feed(panelPages, panelSelected)
        panel?.blocked = entryBlocked
    }
    DisposableEffect(panel, draft) {
        panel?.draftSink = { text -> draft.change(if (draft.text.isBlank()) text else "${draft.text}\n$text") }
        onDispose { panel?.draftSink = null }
    }
    // 转发、撤回都要时间线条目的 id，菜单开着的那条。
    var forwarding by remember { mutableStateOf<String?>(null) }
    var redacting by remember { mutableStateOf<MessageMenuTarget?>(null) }
    // 有东西盖着列表时上推的入口不响应，由核心合成（ADR-0052）。旁聊页没有面板，不报。
    val messageOverlay = menu != null || forwarding != null || redacting != null
    val covers = messageOverlay || timeline.openedConfirmation != null
    if (target.threadRoot == null) {
        LaunchedEffect(covers, roomId) { onAction(Action.SetRoomEntryBlocked(roomId, covers)) }
    }
    Box(Modifier.fillMaxSize()) {
        // 菜单开着时背后的页面模糊（ADR-0029）。Android 12 以下没有模糊，只靠菜单的压暗。
        Box(if (menu != null) Modifier.blur(WontonTheme.size.menuBlur) else Modifier) {
            RoomSurface(
                bar = {
                    RoomBar(
                        target, sideChats, title, isDirect, bot, plugins, timeline.sideChat, onClose,
                        onOpenBotProfile = { onOpenBotProfile(it) },
                    )
                },
            ) { backdrop ->
                Conversation(
                    core, target, sideChats, isDirect, isReadOnly, timeline, composer, draft, onAction, onReject,
                    openMenu,
                    onOpenBackup, backdrop,
                )
            }
        }
        menu?.let { opened ->
            MessageMenu(
                message = opened.message,
                bubble = opened.bubble,
                visible = visible,
                core = core,
                onReply = draft::replyTo,
                onForward = { forwarding = opened.itemId },
                onRedact = { redacting = opened },
                onDismiss = { menu = null },
            )
        }
        forwarding?.let { itemId ->
            ForwardSheet(
                rooms = rooms.filter { it.forwardTarget(roomId) },
                onPick = { row ->
                    onAction(
                        Action.Timeline(
                            TimelineAction.ForwardMessage(target, itemId, row.roomId),
                        ),
                    )
                    onOpenRoom(row.roomId)
                },
                onDismiss = { forwarding = null },
            )
        }
        redacting?.let { opened ->
            RedactConfirm(
                opened.message,
                onConfirm = {
                    onAction(Action.Timeline(TimelineAction.RedactMessage(target, opened.itemId)))
                    redacting = null
                },
                onDismiss = { redacting = null },
            )
        }
        RoomConfirmationSheet(
            timeline.confirmations,
            timeline.openedConfirmation,
            onDismiss = { timeline.openedConfirmation = null },
            onAction = onAction,
        )
        if (sideChats.composing) {
            NewSideChatSheet(
                core,
                roomId,
                onStarted = { root ->
                    sideChats.onComposingChange(false)
                    sideChats.onOpen(root)
                },
                onDismiss = { sideChats.onComposingChange(false) },
            )
        }
    }
}

/**
 * 顶栏。和 Bot 的私聊显示 Bot 的状态头像和名字（ADR-0044），名字下面不写状态文字；支持旁聊的房间带开旁聊的按钮。
 * 旁聊页的标题是旁聊的标题（ADR-0093），状态在顶栏下面一行。
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
) {
    if (target.threadRoot != null) {
        WontonPushedBar(header?.title ?: stringResource(R.string.side_chat), onClose, showsStatus = true)
        return
    }
    val barTitle = if (isDirect) bot?.name ?: title else title
    val barAvatar: (@Composable () -> Unit)? = if (isDirect && bot != null) {
        { BotAvatar(bot, plugins, WontonTheme.size.avatar) }
    } else {
        null
    }
    WontonPushedBar(
        barTitle,
        onClose,
        showsStatus = true,
        showsStatusLine = true,
        avatar = barAvatar,
        actions = {
            if (isDirect && bot != null) {
                Box(Modifier.testTag("bot-profile-open")) {
                    WontonIconButton(
                        WontonIcon.Info, stringResource(R.string.bot_profile_open),
                        { onOpenBotProfile(bot.bot) },
                    )
                }
            }
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
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onOpenBackup: () -> Unit,
    backdrop: RoomBackdrop,
) {
    val density = LocalDensity.current
    var composerHeight by remember { mutableIntStateOf(0) }
    var headerHeight by remember { mutableIntStateOf(0) }
    val barInset = LocalWontonTopInset.current
    val panelVisible = LocalChatPanel.current?.visible == true
    val bottomInsets = if (panelVisible) WindowInsets.ime else WindowInsets.ime.union(WindowInsets.navigationBars)
    Box(Modifier.fillMaxSize().windowInsetsPadding(bottomInsets)) {
        CompositionLocalProvider(
            LocalWontonBottomInset provides with(density) { composerHeight.toDp() },
            LocalWontonTopInset provides barInset + with(density) { headerHeight.toDp() },
        ) {
            val failure = timeline.failure
            if (failure != null) {
                Box(Modifier.fillMaxSize().padding(top = LocalWontonTopInset.current)) {
                    ProblemView(failure) { timeline.attempt++ }
                }
            } else {
                Messages(
                    core, target, sideChats, timeline, composer?.outgoing.orEmpty(),
                    showsSender = !isDirect,
                    onAction = onAction,
                    onOpenMenu = onOpenMenu,
                    onReply = { message -> message.replyAction?.let { draft.replyTo(it.message) } },
                    modifier = Modifier.fillMaxSize().recordRoomBackdrop(backdrop),
                )
            }
        }
        Column(Modifier.fillMaxWidth().padding(top = barInset).onSizeChanged { headerHeight = it.height }) {
            timeline.sideChat?.let { SideChatHeaderLine(it.state) }
            if (sideChats.unsupportedHint) NoSideChatsHint()
        }
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .roomBackdrop(backdrop, top = false)
                .onSizeChanged { composerHeight = it.height },
        ) {
            timeline.notice?.let { KeyNoticeRow(it, onOpenBackup) }
            TransfersStrip(target.roomId, core)
            RoomConfirmationRow(timeline.confirmations) { timeline.openedConfirmation = it }
            if (!isReadOnly) {
                Composer(
                    target = target,
                    draft = draft.text,
                    reply = draft.reply,
                    focusToken = draft.focusToken,
                    composer = composer,
                    core = core,
                    onDraftChange = draft::change,
                    onSent = draft::sent,
                    onReplyCancel = draft::stopReplying,
                    onAction = onAction,
                    onReject = onReject,
                )
            }
        }
    }
}

/**
 * 有消息解不开时的一行说明和入口：账号设过备份就输恢复密钥，没设过就请另一台设备确认。都去备份页。
 * 更新的核心给出的类别没有按钮，说明照常显示。
 */
@Composable
private fun KeyNoticeRow(notice: KeyNotice, onOpenBackup: () -> Unit) {
    Column(
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
    ) {
        WontonNotice(WontonIcon.LockKey, stringResource(R.string.timeline_undecryptable_notice), isError = false) {
            notice.action?.let { action ->
                WontonButton(stringResource(action), onOpenBackup, Modifier.fillMaxWidth(), WontonButtonKind.PRIMARY)
            }
        }
    }
}

/** 转发能去的房间：别的、已经加入、能发的房间。 */
private fun RoomRow.forwardTarget(roomId: String): Boolean =
    this.roomId != roomId && membership == RoomMembership.JOINED && !isReadOnly

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

@Composable
private fun Messages(
    core: ClientCore,
    target: TimelineTarget,
    sideChats: SideChatEntry,
    timeline: TimelineState,
    outgoing: List<OutgoingMessage>,
    showsSender: Boolean,
    onAction: (Action) -> Unit,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    modifier: Modifier,
) {
    val listState = rememberLazyListState()
    val panel = LocalChatPanel.current
    ScrollEffects(listState, timeline, target, sideChats.covered, onAction)
    // 待发消息出现、交出去换成本地回显时，在底部附近的跟着到底：插在第 0 项前面的行，列表按键记位置不会自己露出来。
    val outgoingIds = outgoing.map { it.id }
    LaunchedEffect(outgoingIds) {
        withFrameNanos { }
        if (listState.firstVisibleItemIndex <= outgoing.size + 1) listState.scrollToItem(0)
    }
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    val openCard: (String) -> Unit = { cardId -> timeline.openedConfirmation = cardId }
    // 滚到最上面（最早的一条露出来）时加载更早的消息；加载完条目变多，还在顶上就再加载一页。
    LaunchedEffect(listState, target) {
        snapshotFlow { listState.reachedTop(timeline) }
            .distinctUntilChanged()
            .filter { it }
            .collect { onAction(Action.Timeline(TimelineAction.PaginateBackwards(target))) }
    }
    Box(modifier) {
        LazyColumn(
            modifier = Modifier.fillMaxSize()
                .testTag("room-timeline")
                .panelAction(panel)
                .chatPanelPull(panel, { !listState.canScrollBackward }, !timeline.entryBlocked),
            state = listState,
            reverseLayout = true,
            overscrollEffect = null,
            contentPadding = PaddingValues(
                start = WontonTheme.spacing.pageInset,
                end = WontonTheme.spacing.pageInset,
                top = LocalWontonTopInset.current + WontonTheme.spacing.s8,
                bottom = LocalWontonBottomInset.current + WontonTheme.spacing.s8,
            ),
            verticalArrangement = Arrangement.spacedBy(JOINED_GAP),
        ) {
            // 倒着排：最后点发送的那条在最下面。
            items(outgoing.asReversed(), key = { it.id }) { message ->
                Box(Modifier.padding(top = WontonTheme.spacing.s8 - JOINED_GAP)) {
                    OutgoingRow(message, target.roomId, core, onAction)
                }
            }
            itemsIndexed(newestFirst, key = { _, item -> item.id }) { index, item ->
                // 倒着排：下标大的是更早的一条。
                // 做完了又没什么可写的 Agent 状态行不占地方。
                if (item.isBlankAgentRun) return@itemsIndexed
                val joinsPrevious = item.joins(newestFirst.getOrNull(index + 1))
                val joinsNext = newestFirst.getOrNull(index - 1)?.joins(item) == true
                Box(Modifier.padding(top = if (joinsPrevious) 0.dp else WontonTheme.spacing.s8 - JOINED_GAP)) {
                    TimelineItemView(
                        item,
                        core,
                        showsSender,
                        joinsPrevious,
                        joinsNext,
                        onRetry = { onAction(Action.Timeline(TimelineAction.RetrySend(target, item.id))) },
                        onCancel = { onAction(Action.Timeline(TimelineAction.CancelSend(target, item.id))) },
                        onCardAction = onAction,
                        onOpenMenu = onOpenMenu,
                        onReply = onReply,
                        onOpenAgentCard = openCard,
                        onAnswerAgentChoice = { key ->
                            onAction(Action.AnswerAgentChoice(target.roomId, item.id, key))
                        },
                        onOpenSideChat = sideChats.onOpen,
                    )
                }
            }
            if (timeline.backPagination != BackPagination.REACHED_START) {
                // 列表还空时转圈是第 0 项。它的键不能和有消息时一样：列表按键记位置，消息进来后会跟着转圈跳到最早的一条。
                item(key = if (newestFirst.isEmpty()) "loading" else "paginating") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                }
            }
        }
        ScrollToBottomButton(
            timeline,
            target,
            listState,
            onAction,
            Modifier
                .align(Alignment.BottomEnd)
                .padding(end = WontonTheme.spacing.pageInset, bottom = WontonTheme.spacing.s8),
        )
    }
}

private val JOINED_GAP = 2.dp

/**
 * 和上一条 `earlier` 是同一个人连着发的：并成一组，气泡挨紧，相接的角收小。
 * 中间隔着一行字的不并：这条上方写了时间，或者上一条下面写着已送达、已读。
 */
private fun TimelineItem.joins(earlier: TimelineItem?): Boolean {
    val mine = message
    val theirs = earlier?.message
    return mine != null && theirs != null &&
        mine.senderId == theirs.senderId && !mine.startsTimeGroup && theirs.receipt == null
}

private val TimelineItem.message
    get() = (content as? TimelineContent.Message)?.message

private val TimelineItem.isBlankAgentRun
    get() = (content as? TimelineContent.AgentRun)?.isBlank == true

/// 这条消息带着的确认卡片 ID：Agent 的等待行靠它滚到卡片。
private val TimelineItem.cardId
    get() = ((content as? TimelineContent.Message)?.message?.body as? MessageContent.Confirmation)?.view?.card?.id

internal val TimelineItem.isOwn
    get() = (content as? TimelineContent.Message)?.message?.isOwn == true
