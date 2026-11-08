package app.wonton.ui

import android.util.Log
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
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
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.BotView
import app.wonton.protocol.CoreException
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineDiff
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.filter

private const val TAG = "RoomScreen"

private class TimelineState {
    val items = mutableStateListOf<TimelineItem>()
    var backPagination by mutableStateOf(BackPagination.IDLE)

    /** 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。 */
    var loaded by mutableStateOf(false)
    var failure by mutableStateOf<Problem?>(null)

    /** 打不开时点“重试”加一，按它重新订阅。 */
    var attempt by mutableIntStateOf(0)

    fun apply(diff: TimelineDiff) {
        when (diff) {
            is TimelineDiff.Append -> items.addAll(diff.items)
            TimelineDiff.Clear -> items.clear()
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
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onClose: () -> Unit,
) {
    val timeline = remember(roomId) { TimelineState() }
    LaunchedEffect(core, roomId, timeline.attempt) {
        timeline.failure = null
        try {
            core.timelineUpdates(roomId).collect { update ->
                update.diffs.forEach(timeline::apply)
                timeline.backPagination = update.backPagination
                timeline.loaded = true
            }
            currentCoroutineContext().ensureActive()
            onClose()
        } catch (failure: CoreException) {
            timeline.failure = failure.problem
        }
    }
    val newest = timeline.items.lastOrNull()?.id
    LaunchedEffect(newest) {
        if (newest != null) onAction(Action.MarkRoomRead(roomId))
    }
    val first = remember(core) { core.transfers() }
    val transfers by produceState(first, core) { core.transfersUpdates().collect { value = it } }
    CompositionLocalProvider(LocalFiles provides FilesContext(transfers, onAction, onReject)) {
        RoomScreen(core, roomId, title, isDirect, isReadOnly, bot, plugins, timeline, onAction, onReject, onClose)
    }
}

/** 长按的是哪条消息，它的气泡在屏幕上的位置。 */
private class MessageMenuTarget(val message: TimelineMessage, val bubble: Rect)

@Composable
private fun RoomScreen(
    core: ClientCore,
    roomId: String,
    title: String,
    isDirect: Boolean,
    isReadOnly: Boolean,
    bot: BotView?,
    plugins: PluginsState,
    timeline: TimelineState,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    onClose: () -> Unit,
) {
    var menu by remember { mutableStateOf<MessageMenuTarget?>(null) }
    val openMenu = { message: TimelineMessage, bounds: Rect -> menu = MessageMenuTarget(message, bounds) }
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
    // 草稿在核心里，按房间存：退出房间、重启 App 都还在。进房间读一次；先读回来之前已经打了字就不盖掉。
    // 写只在主人改输入框时发，别把读到的初值也写回去：房间还没同步到本机时读会失败，写一个空串会把存的草稿清掉。
    var draft by rememberSaveable(roomId) { mutableStateOf("") }
    LaunchedEffect(core, roomId) {
        runCatching { core.draft(roomId) }.onSuccess { stored ->
            if (draft.isEmpty()) draft = stored.orEmpty()
        }.onFailure { failure ->
            Log.w(TAG, "could not load the draft for this room", failure)
        }
    }
    val changeDraft = { text: String ->
        draft = text
        onAction(Action.SetDraft(roomId, text))
    }
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
                // 这一页没有底栏，要自己让开手势条；键盘弹出时手势条算在键盘里，不重复让。
                Column(
                    Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars)),
                ) {
                    val failure = timeline.failure
                    if (failure != null) {
                        Box(Modifier.weight(1f).fillMaxWidth()) {
                            ProblemView(failure) { timeline.attempt++ }
                        }
                    } else {
                        val showsSender = !isDirect
                        Messages(core, roomId, timeline, showsSender, onAction, openMenu, Modifier.weight(1f))
                    }
                    TransfersStrip(roomId, core)
                    if (!isReadOnly) Composer(roomId, draft, changeDraft, onAction, onReject)
                }
            }
        }
        menu?.let { target -> MessageMenu(target.message, target.bubble, visible, core) { menu = null } }
    }
}

@Composable
private fun Messages(
    core: ClientCore,
    roomId: String,
    timeline: TimelineState,
    showsSender: Boolean,
    onAction: (Action) -> Unit,
    onOpenMenu: (TimelineMessage, Rect) -> Unit,
    modifier: Modifier,
) {
    val listState = rememberLazyListState()
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    // 新消息插在第 0 项时，列表保持原来看到的那条不动，新消息落在屏幕下面；原本就在底部时滚过去。
    val newest = timeline.items.lastOrNull()?.id
    var followed by remember { mutableStateOf(newest) }
    LaunchedEffect(newest) {
        if (newest == null) return@LaunchedEffect
        // 这里可能先于列表测量新条目运行：那时第 0 项还是旧的那条，滚过去等于没动，新消息留在屏幕下面（实测回复较长时）。
        withFrameNanos { }
        // 一批到了几条时，上次滚到的那条不在第 1 项。滚动被下一条打断时不更新，下一条还会跟上。
        val arrived = newestFirst.indexOfFirst { it.id == followed }.coerceAtLeast(1)
        if (listState.firstVisibleItemIndex <= arrived) listState.animateScrollToItem(0)
        followed = newest
    }
    // 滚到最上面（最早的一条露出来）时加载更早的消息；加载完条目变多，还在顶上就再加载一页。
    LaunchedEffect(listState, roomId) {
        snapshotFlow {
            val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index
            timeline.loaded && last != null && last >= listState.layoutInfo.totalItemsCount - 1 &&
                timeline.backPagination == BackPagination.IDLE
        }
            .distinctUntilChanged()
            .filter { it }
            .collect { onAction(Action.PaginateBackwards(roomId)) }
    }
    LazyColumn(
        modifier.fillMaxWidth(),
        state = listState,
        reverseLayout = true,
        contentPadding = wontonContentPaddingValues(
            horizontal = WontonTheme.spacing.pageInset,
            top = WontonTheme.spacing.s8,
            bottom = WontonTheme.spacing.s8,
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
}

private val JOINED_GAP = 2.dp

/** 同一个人连着发的消息并成一组：气泡挨紧，相接的角收小。 */
private fun TimelineItem.joins(other: TimelineItem?): Boolean {
    val sender = senderId
    return sender != null && sender == other?.senderId
}

private val TimelineItem.senderId
    get() = (content as? TimelineContent.Message)?.message?.senderId
