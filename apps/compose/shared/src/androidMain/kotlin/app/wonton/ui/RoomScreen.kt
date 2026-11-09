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
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Badge
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.BotView
import app.wonton.protocol.CoreException
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineDiff
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.protocol.TimelineScroll
import app.wonton.protocol.TimelineViewport
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.toColor
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch

private const val TAG = "RoomScreen"

private class TimelineState {
    val items = mutableStateListOf<TimelineItem>()
    var backPagination by mutableStateOf(BackPagination.IDLE)

    /** 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。 */
    var loaded by mutableStateOf(false)
    var failure by mutableStateOf<Problem?>(null)

    /** 核心算的位置：在不在底部、离开底部时的新消息数（ADR-0071）。 */
    var viewport by mutableStateOf<TimelineViewport>(TimelineViewport.Bottom)

    /** 第一批的落点：进房间停在这一条上（ADR-0071）。 */
    var anchor by mutableStateOf<String?>(null)

    /** 时间线被清空重建的次数：重建的 Reset 里带新落点，壳要重新摆。 */
    var clears by mutableIntStateOf(0)

    /** 打不开时点“重试”加一，按它重新订阅。 */
    var attempt by mutableIntStateOf(0)

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
            }
        }
    }
}

@Composable
fun RoomRoute(
    core: ClientCore,
    roomId: String,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
    plugins: PluginsState,
    rooms: List<RoomRow>,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onOpenRoom: (String) -> Unit,
    onClose: () -> Unit,
) {
    val timeline = remember(roomId) { TimelineState() }
    LaunchedEffect(core, roomId, timeline.attempt) {
        timeline.failure = null
        try {
            core.timelineUpdates(roomId).collect { update ->
                update.diffs.forEach(timeline::apply)
                timeline.backPagination = update.backPagination
                timeline.viewport = update.viewport
                update.anchor?.let { timeline.anchor = it }
                timeline.loaded = true
            }
            currentCoroutineContext().ensureActive()
            onClose()
        } catch (failure: CoreException) {
            timeline.failure = failure.problem
        }
    }
    val first = remember(core) { core.transfers() }
    val transfers by produceState(first, core) { core.transfersUpdates().collect { value = it } }
    CompositionLocalProvider(LocalFiles provides FilesContext(transfers, onAction, onReject)) {
        RoomScreen(
            core,
            roomId,
            title,
            isDirect,
            isReadOnly,
            bot,
            plugins,
            rooms,
            timeline,
            onAction,
            onReject,
            onOpenRoom,
            onClose,
        )
    }
}

/** 长按的是哪条消息，它的气泡在屏幕上的位置。 */
private class MessageMenuTarget(val itemId: String, val message: TimelineMessage, val bubble: Rect)

@Composable
private fun RoomScreen(
    core: ClientCore,
    roomId: String,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
    plugins: PluginsState,
    rooms: List<RoomRow>,
    timeline: TimelineState,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onOpenRoom: (String) -> Unit,
    onClose: () -> Unit,
) {
    var menu by remember { mutableStateOf<MessageMenuTarget?>(null) }
    val openMenu = { item: TimelineItem, message: TimelineMessage, bounds: Rect ->
        menu = MessageMenuTarget(item.id, message, bounds)
    }
    // 菜单能画在哪儿：窗口减去系统栏和键盘。弹层是另一个窗口，在里面拿不到这些，在这里算好传进去。
    // 横屏时导航栏或挖孔在侧边，左右也要让。
    val windowSize = LocalWindowInfo.current.containerSize
    val density = LocalDensity.current
    val safe = WindowInsets.safeDrawing
    val direction = LocalLayoutDirection.current
    val visible = Rect(
        safe.getLeft(density, direction).toFloat(),
        safe.getTop(density).toFloat(),
        (windowSize.width - safe.getRight(density, direction)).toFloat(),
        (windowSize.height - safe.getBottom(density)).toFloat(),
    )
    val draft = rememberRoomDraft(core, roomId, onAction)
    // 转发、撤回都要时间线条目的 id，菜单开着的那条。
    var forwarding by remember { mutableStateOf<String?>(null) }
    var redacting by remember { mutableStateOf<String?>(null) }
    // 和 Bot 的私聊：顶栏显示 Bot 的状态头像和名字（ADR-0044），名字下面不写状态文字。
    val barTitle = if (isDirect) bot?.name ?: title else title
    val barAvatar: (@Composable () -> Unit)? = if (isDirect && bot != null) {
        { BotAvatar(bot, plugins, WontonTheme.size.avatar) }
    } else {
        null
    }
    Box(Modifier.fillMaxSize()) {
        // 菜单开着时背后的页面模糊（ADR-0029）。Android 12 以下没有模糊，只靠菜单的压暗。
        Box(if (menu != null) Modifier.blur(WontonTheme.size.menuBlur) else Modifier) {
            WontonScreen(bar = { WontonPushedBar(barTitle, onClose, showsStatus = true, avatar = barAvatar) }) {
Conversation(core, roomId, isDirect, isReadOnly, timeline, draft, onAction, onReject, openMenu)
            }
        }
        menu?.let { target ->
            MessageMenu(
                message = target.message,
                bubble = target.bubble,
                visible = visible,
                core = core,
                onReply = draft::replyTo,
                onForward = { forwarding = target.itemId },
                onRedact = { redacting = target.itemId },
                onDismiss = { menu = null },
            )
        }
        forwarding?.let { itemId ->
            ForwardSheet(
                rooms = rooms.filter { it.forwardTarget(roomId) },
                onPick = { row ->
                    onAction(Action.ForwardMessage(roomId, itemId, row.roomId))
                    onOpenRoom(row.roomId)
                },
                onDismiss = { forwarding = null },
            )
        }
        redacting?.let { itemId ->
            RedactConfirm(
                onConfirm = {
                    onAction(Action.RedactMessage(roomId, itemId))
                    redacting = null
                },
                onDismiss = { redacting = null },
            )
        }
    }
}

/** 消息列表、传输条和输入栏。打开失败时列表位置换成原因和重试。 */
@Composable
private fun Conversation(
    core: ClientCore,
    roomId: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    timeline: TimelineState,
    draft: RoomDraft,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
) {
    // 这一页没有底栏，要自己让开手势条；键盘弹出时手势条算在键盘里，不重复让。
    // 房间列表倒着排、贴底，稳态下旧消息正对着顶栏：让位不能像普通滚动页那样走 contentPadding
    // （随滚动消失，消息仍画到标题和状态栏上），视口本身要从顶栏下面开始。
    Column(
        Modifier.fillMaxSize()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars))
            .padding(top = LocalWontonTopInset.current),
    ) {
        val failure = timeline.failure
        if (failure != null) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                ProblemView(failure) { timeline.attempt++ }
            }
        } else {
            Messages(
                core,
                roomId,
                timeline,
                showsSender = !isDirect,
                onAction = onAction,
                onOpenMenu = onOpenMenu,
                onReply = { message -> message.replyAction?.let { draft.replyTo(it.message) } },
                modifier = Modifier.weight(1f),
            )
        }
        TransfersStrip(roomId, core)
        if (!isReadOnly) {
            Composer(
                roomId = roomId,
                draft = draft.text,
                reply = draft.reply,
                focusToken = draft.focusToken,
                core = core,
                onDraftChange = draft::change,
                onSent = draft::sent,
                onReplySent = draft::replySent,
                onReplyCancel = draft::stopReplying,
                onAction = onAction,
                onReject = onReject,
            )
        }
    }
}

/** 转发能去的房间：别的、已经加入、能发的房间。 */
private fun RoomRow.forwardTarget(roomId: String): Boolean =
    this.roomId != roomId && membership == RoomMembership.JOINED && !isReadOnly

/** 撤回对所有人生效，发出去之前先确认。 */
@Composable
private fun RedactConfirm(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    WontonDialog(
        title = stringResource(R.string.message_delete_everyone_title),
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
    roomId: String,
    timeline: TimelineState,
    showsSender: Boolean,
    onAction: (Action) -> Unit,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    modifier: Modifier,
) {
    val listState = rememberLazyListState()
    ScrollEffects(listState, timeline, roomId, onAction)
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    // 滚到最上面（最早的一条露出来）时加载更早的消息；加载完条目变多，还在顶上就再加载一页。
    LaunchedEffect(listState, roomId) {
        snapshotFlow { listState.reachedTop(timeline) }
            .distinctUntilChanged()
            .filter { it }
            .collect { onAction(Action.PaginateBackwards(roomId)) }
    }
    Box(modifier) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            state = listState,
            reverseLayout = true,
            // 顶栏让位已垫在视口上（上面的 Column），这里不再叠加；底部照旧让开手势条。
            contentPadding = PaddingValues(
                start = WontonTheme.spacing.pageInset,
                end = WontonTheme.spacing.pageInset,
                top = WontonTheme.spacing.s8,
                bottom = LocalWontonBottomInset.current + WontonTheme.spacing.s8,
            ),
            verticalArrangement = Arrangement.spacedBy(JOINED_GAP),
        ) {
            itemsIndexed(newestFirst, key = { _, item -> item.id }) { index, item ->
                // 倒着排：下标大的是更早的一条。
                val joinsPrevious = item.joins(newestFirst.getOrNull(index + 1))
                val joinsNext = item.joins(newestFirst.getOrNull(index - 1))
                Box(Modifier.padding(top = if (joinsPrevious) 0.dp else WontonTheme.spacing.s8 - JOINED_GAP)) {
                    TimelineItemView(
                        item,
                        core,
                        showsSender,
                        joinsPrevious,
                        joinsNext,
                        onRetry = { onAction(Action.RetrySend(roomId, item.id)) },
                        onCancel = { onAction(Action.CancelSend(roomId, item.id)) },
                        onCardAction = onAction,
                        onOpenMenu = onOpenMenu,
                    onReply = onReply,
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
            roomId,
            listState,
            onAction,
            Modifier
                .align(Alignment.BottomEnd)
                .padding(end = WontonTheme.spacing.pageInset, bottom = WontonTheme.spacing.s8),
        )
    }
}

/// 进房间的落点、新消息的跟随、滚动方向的报告、只在底部标已读（ADR-0071、#159）。
@Composable
private fun ScrollEffects(
    listState: LazyListState,
    timeline: TimelineState,
    roomId: String,
    onAction: (Action) -> Unit,
) {
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    // 新消息插在第 0 项时，列表保持原来看到的那条不动，新消息落在屏幕下面；原本就在底部时滚过去。
    val newest = timeline.items.lastOrNull()?.id
    var followed by remember { mutableStateOf(newest) }
    // 进房间的落点摆好之前：不跟随、不报方向、不标已读（ADR-0071）。时间线清空重建后闸门重开，按新落点重摆。
    var placed by remember(timeline.clears) { mutableStateOf(false) }
    LaunchedEffect(newest, timeline.clears) {
        if (newest == null) return@LaunchedEffect
        // 这里可能先于列表测量新条目运行：那时第 0 项还是旧的那条，滚过去等于没动，新消息留在屏幕下面（实测回复较长时）。
        withFrameNanos { }
        if (placed) {
            followNewest(listState, newestFirst, followed)
        } else {
            placeAnchor(listState, newestFirst, timeline, roomId, newest, onAction)
        }
        followed = newest
        placed = true
    }
    // 滚动跨过底部时报告方向（ADR-0071），核心按它算位置和新消息数；摆落点前不报。
    LaunchedEffect(roomId) {
        snapshotFlow { listState.isAtBottom() }
            .drop(1)
            .distinctUntilChanged()
            .collect { atBottom ->
                if (placed) {
                    val scroll = if (atBottom) TimelineScroll.BOTTOM else TimelineScroll.AWAY
                    onAction(Action.SetTimelineScroll(roomId, scroll))
                }
            }
    }
    // 只有停在底部才标已读；翻上去读历史时不标，回到底部再标。停在底部不动时来的新消息也要标：
    // 快照里带上最新一条的 id，它变了就再看一眼是否在底部。
    LaunchedEffect(roomId) {
        snapshotFlow {
            Triple(placed, listState.isAtBottom(), timeline.items.lastOrNull()?.id)
        }.collect { (placed, atBottom, newest) ->
            if (placed && atBottom && newest != null) {
                if (timeline.loaded) onAction(Action.MarkRoomRead(roomId))
            }
        }
    }
}

/// 进房间停在上次看的位置（ADR-0071）：落点行先贴到底边，量出它的高度，再把它顶到视野顶部，它后面的新消息
/// 接着往下排。落点不在这一页里或没给，停在最新一条。落点不是最新一条，就是停在别处：报 Away，核心把按钮和
/// 新消息数算出来。
private suspend fun placeAnchor(
    listState: LazyListState,
    newestFirst: List<TimelineItem>,
    timeline: TimelineState,
    roomId: String,
    newest: String,
    onAction: (Action) -> Unit,
) {
    val anchor = timeline.anchor
    val index = newestFirst.indexOfFirst { it.id == anchor }
    if (anchor == null || index <= 0) {
        // 没给落点、落点不在这一页里、或落点就是最新一条：都停在最新一条，不顶行（顶了会留出一片空白，
        // 还会让"停在底部"判定变假，给读满的房间冒出个未读 0 的按钮）。
        listState.scrollToItem(0)
        return
    }
    listState.scrollToItem(index)
    // 等一帧量出落点行的高度；量不到（行还没画出来）就停在贴底的位置。
    withFrameNanos { }
    val height = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }?.size
    if (height != null && height > 0) {
        listState.scrollToItem(index, listState.layoutInfo.viewportSize.height - height)
    }
    if (anchor != newest) {
        onAction(Action.SetTimelineScroll(roomId, TimelineScroll.AWAY))
    }
}

/// 新消息的跟随：一批到了几条时，上次滚到的那条不在第 1 项。新到的消息里有自己发的就回到底部，翻着也回。
private suspend fun followNewest(
    listState: LazyListState,
    newestFirst: List<TimelineItem>,
    followed: String?,
) {
    val arrived = newestFirst.indexOfFirst { it.id == followed }.coerceAtLeast(1)
    val ownArrived = newestFirst.take(arrived).any { it.isOwn }
    val at = listState.firstVisibleItemIndex
    val offset = listState.firstVisibleItemScrollOffset
    // 没翻走才跟随：比上次滚到的那条更新，或者正在它上面一步没滑出去。翻上去读历史就不动。
    // 跟随的滚动被下一条打断时，这行不执行，下一条还会跟上。
    val atFollowed = at == arrived && offset == 0
    if (ownArrived || at < arrived || atFollowed) listState.animateScrollToItem(0)
}

/** 停在别处时的“回到底部”：数字是离开底部后到的消息数（ADR-0071），没有就不写数字。 */
@Composable
private fun ScrollToBottomButton(
    timeline: TimelineState,
    roomId: String,
    listState: LazyListState,
    onAction: (Action) -> Unit,
    modifier: Modifier,
) {
    val scope = rememberCoroutineScope()
    AnimatedVisibility(
        visible = timeline.viewport is TimelineViewport.Away,
        enter = fadeIn() + scaleIn(initialScale = BUTTON_APPEAR_SCALE),
        exit = fadeOut() + scaleOut(targetScale = BUTTON_APPEAR_SCALE),
        modifier = modifier,
    ) {
        val away = timeline.viewport as? TimelineViewport.Away
        val count = away?.newMessages?.toInt() ?: 0
        val label = if (count > 0) {
            stringResource(R.string.room_scroll_to_bottom_count, count)
        } else {
            stringResource(R.string.room_scroll_to_bottom)
        }
        FloatingActionButton(
            onClick = {
                onAction(Action.SetTimelineScroll(roomId, TimelineScroll.BOTTOM))
                scope.launch { listState.animateScrollToItem(0) }
            },
            modifier = Modifier
                .size(WontonTheme.size.floatingActionButton)
                .semantics { contentDescription = label },
            containerColor = WontonTheme.colors.accentFill.toColor(),
            contentColor = WontonTheme.colors.accentInkOnFill.toColor(),
        ) {
            // 角标叠在圆钮外面：FAB 的 Surface 会按圆形裁剪内容，放进 BadgedBox 超出的部分会被切掉。
            Box {
                Icon(Icons.Filled.KeyboardArrowDown, contentDescription = null)
                if (count > 0) {
                    Badge(
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .offset(x = 6.dp, y = (-6).dp),
                    ) {
                        Text(if (count > BADGE_COUNT_MAX) "$BADGE_COUNT_MAX+" else count.toString())
                    }
                }
            }
        }
    }
}

/** 按钮随 Away 进出时的起始缩放。 */
private const val BUTTON_APPEAR_SCALE = 0.6f

/** 角标数字超过这个数就写“99+”。 */
private const val BADGE_COUNT_MAX = 99

private val JOINED_GAP = 2.dp

/** 第 0 项是最新的一条，贴着底边、没滑出去才算停在底部。 */
private fun LazyListState.isAtBottom(): Boolean =
    firstVisibleItemIndex == 0 && firstVisibleItemScrollOffset == 0

/** 最早的一条露出来了，该向前翻页。 */
private fun LazyListState.reachedTop(timeline: TimelineState): Boolean {
    val last = layoutInfo.visibleItemsInfo.lastOrNull()?.index
    return timeline.loaded && last != null && last >= layoutInfo.totalItemsCount - 1 &&
        timeline.backPagination == BackPagination.IDLE
}

/** 同一个人连着发的消息并成一组：气泡挨紧，相接的角收小。 */
private fun TimelineItem.joins(other: TimelineItem?): Boolean {
    val sender = senderId
    return sender != null && sender == other?.senderId
}

private val TimelineItem.senderId
    get() = (content as? TimelineContent.Message)?.message?.senderId

private val TimelineItem.isOwn
    get() = (content as? TimelineContent.Message)?.message?.isOwn == true
