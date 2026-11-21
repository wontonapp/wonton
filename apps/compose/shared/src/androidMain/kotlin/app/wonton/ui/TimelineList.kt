package app.wonton.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.ChatAction
import app.wonton.protocol.JumpState
import app.wonton.protocol.OutgoingMessage
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch

/**
 * 消息列表、传输条和输入栏里的列表那一半：倒着排，最新的一条贴着底部。跳转、回到底部、日期胶囊和分页都在这里，
 * 每条交给 [TimelineRow] 画。
 */
@Composable
internal fun TimelineList(
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
    menuOpen: Boolean = false,
    headerHeight: Dp = 0.dp,
    onScroll: () -> Unit = {},
) {
    val listState = rememberLazyListState()
    ScrollEffects(listState, timeline, target, sideChats.covered, onAction)
    FollowNewItems(listState, outgoing)
    // 翻时间线时先把命令列表收起来（ADR-0109 第 7 节），列表让出来。
    LaunchedEffect(listState) {
        snapshotFlow { listState.isScrollInProgress }.filter { it }.collect { onScroll() }
    }
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    val openCard: (String) -> Unit = { cardId -> timeline.openedConfirmation = cardId }
    val pill = rememberDatePill(listState, newestFirst)
    val inset = LocalWontonTopInset.current
    PaginateOnTop(listState, timeline, target, onAction)
    Box(modifier) {
        LazyColumn(
            modifier = Modifier.fillMaxSize().testTag("room-timeline"),
            state = listState,
            reverseLayout = true,
            overscrollEffect = null,
            contentPadding = PaddingValues(
                start = WontonTheme.spacing.pageInset,
                end = WontonTheme.spacing.pageInset,
                top = 0.dp,
                bottom = LocalWontonBottomInset.current + WontonTheme.spacing.s8,
            ),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.runGap),
        ) {
            // 每次列表内容重建都重算；用 remember 记在函数体里时，条目晚到的那次重排看不到新日期。
            val dayMarks = dayMarksFor(newestFirst)
            // 倒着排：最后点发送的那条在最下面。
            items(outgoing.asReversed(), key = { it.id }) { message ->
                Box(Modifier.padding(top = WontonTheme.spacing.groupGap - WontonTheme.spacing.runGap)) {
                    OutgoingRow(message, target.roomId, core, onAction)
                }
            }
            itemsIndexed(newestFirst, key = { _, item -> item.id }) { index, item ->
                val day = dayMarks.getOrNull(index)
                // 做完了又没什么可写的 Agent 状态行不占地方。
                if (item.isBlankAgentRun) return@itemsIndexed
                Column {
                    // 跨日的那条上方放一行固定的日期：它自己占一行，不和气泡重叠。
                    day?.let { label -> DaySeparator(label) }
                    // 倒着排：下标大的是更早的一条。
                    TimelineRow(
                        item,
                        previous = newestFirst.getOrNull(index + 1),
                        next = newestFirst.getOrNull(index - 1),
                        core = core,
                        target = target,
                        showsSender = showsSender,
                        timeline = timeline,
                        sideChats = sideChats,
                        onAction = onAction,
                        onOpenMenu = onOpenMenu,
                        onReply = onReply,
                        openCard = openCard,
                    )
                }
            }
            // 顶栏空出的这段也要能滚：放进 contentPadding 时，倒排的列表翻到最早一条会把这段吃掉，
            // 第一条正好压在顶栏底下。做成一格列表项，翻到头时它停在顶栏下面。
            item(key = "top-gap") { Spacer(Modifier.height(inset + WontonTheme.spacing.s8)) }
            if (timeline.backPagination != BackPagination.REACHED_START) {
                // 列表还空时转圈是第 0 项。它的键不能和有消息时一样：列表按键记位置，消息进来后会跟着转圈跳到最早的一条。
                item(key = if (newestFirst.isEmpty()) "loading" else "paginating") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                }
            }
        }
        DatePill(
            pill.held && pill.text != null,
            pill.text.orEmpty(),
            Modifier.align(Alignment.TopCenter).padding(
                // 有状态行时列表已经让开了它，胶囊只躲顶栏那一层；没有状态行时躲顶栏本身。
                top = if (headerHeight > 0.dp) {
                    WontonTheme.spacing.s4
                } else {
                    LocalWontonTopInset.current + WontonTheme.spacing.s4
                },
            ),
        )
        // 跳转找不到那条消息时轻轻说一声，列表不动。
        DatePill(
            timeline.jump == JumpState.UNAVAILABLE,
            stringResource(R.string.timeline_jump_unavailable),
            Modifier.align(Alignment.BottomCenter).padding(bottom = WontonTheme.spacing.s8),
        )
        ScrollToBottomButton(
            timeline,
            target,
            listState,
            onAction,
            hidden = menuOpen,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                // 输入框压在时间线底部，按钮不跟着抬就被挡在它后面（点下去打到的是输入框）。
                .padding(
                    end = WontonTheme.spacing.pageInset,
                    bottom = LocalWontonBottomInset.current + WontonTheme.spacing.s8,
                ),
        )
    }
}

/** 待发消息出现、交出去换成本地回显时，在底部附近的跟着到底：插在第 0 项前面的行，列表按键记位置不会自己露出来。 */
@Composable
private fun FollowNewItems(listState: LazyListState, outgoing: List<OutgoingMessage>) {
    val outgoingIds = outgoing.map { it.id }
    LaunchedEffect(outgoingIds) {
        withFrameNanos { }
        if (listState.firstVisibleItemIndex <= outgoing.size + 1) listState.scrollToItem(0)
    }
}

/** 滚到最上面（最早的一条露出来）时加载更早的消息；加载完条目变多，还在顶上就再加载一页。 */
@Composable
private fun PaginateOnTop(
    listState: LazyListState,
    timeline: TimelineState,
    target: TimelineTarget,
    onAction: (Action) -> Unit,
) {
    LaunchedEffect(listState, target) {
        snapshotFlow { listState.reachedTop(timeline) }
            .distinctUntilChanged()
            .filter { it }
            .collect { onAction(Action.Timeline(TimelineAction.PaginateBackwards(target))) }
    }
}

/** 浮在最上面的日期胶囊：滚动中跟着最上面那条的日期，停下再留一会儿淡出。 */
private class DatePillState {
    var text by mutableStateOf<String?>(null)
    var held by mutableStateOf(false)
}

@Composable
private fun rememberDatePill(listState: LazyListState, newestFirst: List<TimelineItem>): DatePillState {
    val pill = remember { DatePillState() }
    LaunchedEffect(listState, newestFirst) {
        snapshotFlow { newestFirst.getOrNull(listState.topVisibleItem)?.timestampMs }
            .distinctUntilChanged().collect { stamp -> if (stamp != null) pill.text = dateLabel(stamp) }
    }
    LaunchedEffect(listState) {
        var hold: Job? = null
        snapshotFlow { listState.isScrollInProgress }.distinctUntilChanged().collect { scrolling ->
            hold?.cancel()
            if (scrolling) {
                pill.held = true
            } else {
                hold = launch {
                    delay(WontonTheme.motion.datePillHold.toLong())
                    pill.held = false
                }
            }
        }
    }
    return pill
}

/**
 * 列表里的一条：分组按前后两条算，尾巴、圆角和间距跟着组走；跳过去的落点居中高亮（ADR-0109）。
 * [previous] 是更早的一条，[next] 是更新的那条。
 */
@Composable
private fun TimelineRow(
    item: TimelineItem,
    previous: TimelineItem?,
    next: TimelineItem?,
    core: ClientCore,
    target: TimelineTarget,
    showsSender: Boolean,
    timeline: TimelineState,
    sideChats: SideChatEntry,
    onAction: (Action) -> Unit,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    openCard: (String) -> Unit,
) {
    val joinsPrevious = item.joins(previous)
    Box(
        Modifier.padding(
            top = if (joinsPrevious) 0.dp else WontonTheme.spacing.groupGap - WontonTheme.spacing.runGap,
        ),
    ) {
        TimelineItemView(
            item,
            core,
            showsSender,
            joinsPrevious,
            next?.joins(item) == true,
            onRetry = { onAction(Action.Timeline(TimelineAction.RetrySend(target, item.id))) },
            onCancel = { onAction(Action.Timeline(TimelineAction.CancelSend(target, item.id))) },
            onCardAction = onAction,
            onOpenMenu = onOpenMenu,
            onReply = onReply,
            onOpenAgentCard = openCard,
            onAnswerAgentChoice = { key ->
                onAction(Action.Chat(ChatAction.AnswerAgentChoice(target.roomId, item.id, key)))
            },
            onOpenSideChat = sideChats.onOpen,
            onJumpQuote = { eventId ->
                onAction(Action.Timeline(TimelineAction.JumpToMessage(target, eventId, item.id)))
            },
            onAnswerChoice = { key ->
                onAction(Action.Chat(ChatAction.AnswerAgentChoice(target.roomId, item.id, key)))
            },
            focused = timeline.focus?.itemId == item.id,
            emphasize = timeline.focus?.highlight == true,
            flushesPrevious = previous?.flushesWith(next = item) == true,
            flushesNext = item.flushesWith(next = next),
        )
    }
}

/**
 * 每个下标上方要不要来一行日期；跨日的那条才有。列表是倒排的（下标 0 贴着底部），屏幕上往下是越来越新：
 * 一条和它正上方那条不是同一天（或者它就在最顶上、上面没有消息）时，分隔画在它上面、写它自己的日期，
 * 也就是它下面那条消息的日期。
 */
private fun dayMarksFor(items: List<TimelineItem>): Array<String?> {
    // 没有时间的旁注跟着下面（更新）的那条算同一天，下标越小越新。
    val days = arrayOfNulls<String>(items.size)
    var newer: String? = null
    for (index in items.indices) {
        newer = items[index].timestampMs?.let(::dateLabel) ?: newer
        days[index] = newer
    }
    val marks = arrayOfNulls<String>(items.size)
    for (index in items.indices) {
        val day = days[index]
        if (day != null && day != days.getOrNull(index + 1)) marks[index] = day
    }
    return marks
}

/** 日期分隔：一行居中、自己占高度。 */
@Composable
private fun DaySeparator(label: String) {
    Box(Modifier.fillMaxWidth().testTag("day-separator"), contentAlignment = Alignment.Center) {
        Text(label, style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
    }
}

/** 浮在列表上的小胶囊（日期、跳转失败的提示）。 */
@Composable
private fun DatePill(visible: Boolean, text: String, modifier: Modifier) {
    AnimatedVisibility(visible, modifier.testTag("date-pill"), fadeIn(), fadeOut()) {
        Text(
            text,
            Modifier
                .background(
                    WontonTheme.colors.surfaceSubtle.toColor().copy(alpha = PILL_ALPHA),
                    RoundedCornerShape(WontonTheme.radii.pill),
                )
                .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
            style = WontonTheme.typography.caption,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 浮在最上面的日期胶囊的半透明底。 */
private const val PILL_ALPHA = 0.9f

/** 最上面那条的日期，胶囊里写这个。 */
private fun dateLabel(timestampMs: Long): String =
    DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(timestampMs))

/** 这条的发送时间；旁注类的条目没有。 */
private val TimelineItem.timestampMs: Long?
    get() = when (val body = content) {
        is TimelineContent.Message -> body.message.timestampMs.toLong()
        is TimelineContent.AgentRun -> body.timestampMs.toLong()
        else -> null
    }

/** 视觉上最上面那条在倒排列表里的下标：`reverseLayout` 下是最后一个可见项。 */
private val LazyListState.topVisibleItem: Int
    get() = layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0

private val TimelineItem.isBlankAgentRun
    get() = (content as? TimelineContent.AgentRun)?.isBlank == true
