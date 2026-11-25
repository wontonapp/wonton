package app.wonton.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineScroll
import app.wonton.protocol.TimelineTarget
import app.wonton.protocol.TimelineViewport
import app.wonton.ui.design.ScrollToBottomAction
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/// 进房间停在上次看的位置（ADR-0071）：落点行先贴到底边，量出它的高度，再把它顶到视野顶部，它后面的新消息
/// 接着往下排。落点不在这一页里或没给，停在最新一条。落点不是最新一条，就是停在别处：报 Away，核心把按钮和
/// 新消息数算出来。
private suspend fun placeAnchor(
    listState: LazyListState,
    newestFirst: List<TimelineItem>,
    timeline: TimelineState,
    target: TimelineTarget,
    newest: String,
    onAction: (Action) -> Unit,
): Boolean {
    val anchor = timeline.anchor
    val index = newestFirst.indexOfFirst { it.id == anchor }
    val atLatest = anchor == null || index <= 0
    if (atLatest) {
        // 没给落点、落点不在这一页里、或落点就是最新一条：都停在最新一条，不顶行（顶了会留出一片空白，
        // 还会让"停在底部"判定变假，给读满的房间冒出个未读 0 的按钮）。
        listState.scrollToItem(0)
    } else {
        listState.scrollToItem(index)
        // 等一帧量出落点行的高度；量不到（行还没画出来）就停在贴底的位置。
        withFrameNanos { }
        val height = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }?.size
        if (height != null && height > 0) {
            listState.scrollToItem(index, listState.layoutInfo.viewportSize.height - height)
        }
        if (anchor != newest) {
            onAction(scrollAction(target, TimelineScroll.AWAY))
        }
    }
    return atLatest || anchor == newest
}

/// 进房间的落点、新消息的跟随、滚动方向的报告、只在底部标已读（ADR-0071、#159）。`covered`：旁聊页盖在这条主对话
/// 上面，主对话看不见，不标已读（ADR-0093）。
@Composable
internal fun ScrollEffects(
    listState: LazyListState,
    timeline: TimelineState,
    target: TimelineTarget,
    covered: Boolean,
    onAction: (Action) -> Unit,
) {
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    // 进房间的落点摆好之前：不跟随、不报方向、不标已读（ADR-0071）。时间线清空重建后闸门重开，按新落点重摆。
    val placed = remember(timeline.clears) { mutableStateOf(false) }
    val following = remember(target) { mutableStateOf(true) }
    JumpToFocus(listState, timeline, newestFirst, target, onAction, onLanded = { timeline.jumpQuote = null }) {
        following.value = false
    }
    PlaceAtAnchor(listState, timeline, newestFirst, target, placed, following, onAction)
    ReportAtBottom(listState, timeline, target, covered, placed, following, onAction)
}

/**
 * 跳转落到的位置：滚到中间，停在不跟随的位置，让“返回”按钮出来。
 * 只按核心每次跳转加一的序号重启：同一条原消息连点两下也要重新滚一次。
 * 跳转途中来的新批次会改写焦点，拿焦点当键就会被取消在半路；所以这里只等这一跳的落点自己出现
 * （翻页找到以后才来），中途的批次不再打断。被新一跳取消时不动 `jumpQuote`，它归新那一跳。
 * 先画一帧再动手，装转圈的那一帧里落在旧位置上的点击，滚到的还是这一次的落点。
 */
@Composable
private fun JumpToFocus(
    listState: LazyListState,
    timeline: TimelineState,
    newestFirst: List<TimelineItem>,
    target: TimelineTarget,
    onAction: (Action) -> Unit,
    onLanded: () -> Unit,
    stopFollowing: () -> Unit,
) {
    LaunchedEffect(timeline.focusRevision) {
        val revision = timeline.focusRevision
        val id = snapshotFlow { timeline.focus?.itemId }.filterNotNull().first()
        try {
            stopFollowing()
            withFrameNanos { }
            val index = newestFirst.indexOfFirst { it.id == id }
            if (index >= 0) {
                listState.scrollToItem(index)
                // 条目刚变可见时可能还没量出高度，等它进布局再居中；超过几帧就别再等。
                var item = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }
                var waited = 0
                while (item == null && waited < CENTER_FRAMES) {
                    withFrameNanos { }
                    item = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }
                    waited++
                }
                if (item != null) {
                    // 反序列表里正偏移是往上滚过条目：长的往中间挪要正数，短的往回补要负数。
                    listState.scrollToItem(index, (item.size - listState.layoutInfo.viewportSize.height) / 2)
                    if (!listState.isAtBottom()) onAction(scrollAction(target, TimelineScroll.AWAY))
                }
            }
        } finally {
            // 只有还在这一次跳转里才收：新一跳已经把 `jumpQuote` 换成它自己的了。
            if (timeline.focusRevision == revision) onLanded()
        }
    }
}

/** 落点居中前最多等几帧，等条目量出高度。 */
private const val CENTER_FRAMES = 5

/** 时间线变了：头一次摆落点，之后跟着新消息到底（ADR-0071）。 */
@Composable
private fun PlaceAtAnchor(
    listState: LazyListState,
    timeline: TimelineState,
    newestFirst: List<TimelineItem>,
    target: TimelineTarget,
    placed: MutableState<Boolean>,
    following: MutableState<Boolean>,
    onAction: (Action) -> Unit,
) {
    val newest = timeline.items.lastOrNull()?.id
    LaunchedEffect(timeline.revision, timeline.clears) {
        if (newest == null) return@LaunchedEffect
        withFrameNanos { }
        if (!placed.value) {
            following.value = placeAnchor(listState, newestFirst, timeline, target, newest, onAction)
        } else if (timeline.bottomRequest != 0 && timeline.bottomRequest != timeline.handledBottomRequest) {
            listState.scrollToItem(0)
        } else if (following.value && !listState.isScrollInProgress) {
            listState.scrollToItem(0)
        }
        timeline.handledBottomRequest = timeline.bottomRequest
        placed.value = true
    }
}

/**
 * 滚动跨过底部时报告方向（ADR-0071），核心按它算位置和新消息数；摆落点前不报。
 * 非手势的插入和行变高不能被当成主人翻走；真正的滚动在交互观察里报告。
 */
@Composable
private fun ReportAtBottom(
    listState: LazyListState,
    timeline: TimelineState,
    target: TimelineTarget,
    covered: Boolean,
    placed: MutableState<Boolean>,
    following: MutableState<Boolean>,
    onAction: (Action) -> Unit,
) {
    LaunchedEffect(target, covered) {
        snapshotFlow {
            Triple(placed.value, listState.isAtBottom(), timeline.items.lastOrNull()?.id)
        }.distinctUntilChanged().collect { (ready, atBottom, latest) ->
            if (ready && atBottom) {
                following.value = true
                onAction(scrollAction(target, TimelineScroll.BOTTOM))
                if (latest != null && !covered) onAction(markReadAction(target))
            }
        }
    }
    LaunchedEffect(target) {
        snapshotFlow { listState.isScrollInProgress to listState.isAtBottom() }
            .distinctUntilChanged().collect { (scrolling, atBottom) ->
                if (placed.value && scrolling && !atBottom) {
                    following.value = false
                    onAction(scrollAction(target, TimelineScroll.AWAY))
                }
            }
    }
}

/** 停在别处时的“回到底部”：数字是离开底部后到的消息数（ADR-0071），没有就不写数字。 */
@Composable
internal fun ScrollToBottomButton(
    timeline: TimelineState,
    target: TimelineTarget,
    listState: LazyListState,
    onAction: (Action) -> Unit,
    modifier: Modifier,
    hidden: Boolean = false,
) {
    val scope = rememberCoroutineScope()
    AnimatedVisibility(
        visible = !hidden && timeline.viewport is TimelineViewport.Away,
        enter = fadeIn() + scaleIn(initialScale = BUTTON_APPEAR_SCALE),
        exit = fadeOut() + scaleOut(targetScale = BUTTON_APPEAR_SCALE),
        modifier = modifier,
    ) {
        ScrollToBottomAction(
            count = (timeline.viewport as? TimelineViewport.Away)?.newMessages?.toInt() ?: 0,
            onClick = {
                // 跳转过去还没看回去过：先退回跳走的那条，退到没得退了才回最新的一条。
                // 退回也是一次跳转，先报序号，落点效果才认得出这一次（它会等焦点）。
                if (timeline.backTo != null) {
                    timeline.jumpSeq++
                    onAction(Action.Timeline(TimelineAction.RegisterFocusRevision(target, timeline.jumpSeq.toUInt())))
                    onAction(Action.Timeline(TimelineAction.JumpBack(target)))
                } else {
                    scope.launch {
                        listState.animateScrollToItem(0)
                        onAction(scrollAction(target, TimelineScroll.BOTTOM))
                        onAction(markReadAction(target))
                    }
                }
            },
        )
    }
}

/// 按钮随 Away 进出时的起始缩放。
private const val BUTTON_APPEAR_SCALE = 0.6f

/** 第 0 项是最新的一条，贴着底边、没滑出去才算停在底部。 */
private fun LazyListState.isAtBottom(): Boolean =
    firstVisibleItemIndex == 0 && firstVisibleItemScrollOffset == 0

/** 最早的一条露出来了，该向前翻页。 */
internal fun LazyListState.reachedTop(timeline: TimelineState): Boolean {
    val last = layoutInfo.visibleItemsInfo.lastOrNull()?.index
    return timeline.loaded && last != null && last >= layoutInfo.totalItemsCount - 1 &&
        timeline.backPagination == BackPagination.IDLE
}

private fun scrollAction(target: TimelineTarget, scroll: TimelineScroll) =
    Action.Timeline(TimelineAction.SetTimelineScroll(target, scroll))

private fun markReadAction(target: TimelineTarget) = Action.Timeline(TimelineAction.MarkRead(target))
