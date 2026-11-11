package app.wonton.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.Badge
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineScroll
import app.wonton.protocol.TimelineViewport
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/// 进房间的落点、新消息的跟随、滚动方向的报告、只在底部标已读（ADR-0071、#159）。
@Composable
internal fun ScrollEffects(
    listState: LazyListState,
    timeline: TimelineState,
    roomId: String,
    onAction: (Action) -> Unit,
) {
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    val newest = timeline.items.lastOrNull()?.id
    var following by remember(roomId) { mutableStateOf(true) }
    // 进房间的落点摆好之前：不跟随、不报方向、不标已读（ADR-0071）。时间线清空重建后闸门重开，按新落点重摆。
    var placed by remember(timeline.clears) { mutableStateOf(false) }
    LaunchedEffect(timeline.revision, timeline.clears) {
        if (newest == null) return@LaunchedEffect
        withFrameNanos { }
        if (!placed) {
            following = placeAnchor(listState, newestFirst, timeline, roomId, newest, onAction)
        } else if (timeline.bottomRequest != 0 && timeline.bottomRequest != timeline.handledBottomRequest) {
            listState.scrollToItem(0)
        } else if (following && !listState.isScrollInProgress) {
            listState.scrollToItem(0)
        }
        timeline.handledBottomRequest = timeline.bottomRequest
        placed = true
    }
    // 滚动跨过底部时报告方向（ADR-0071），核心按它算位置和新消息数；摆落点前不报。
    LaunchedEffect(roomId) {
        snapshotFlow {
            Triple(placed, listState.isAtBottom(), timeline.items.lastOrNull()?.id)
        }.distinctUntilChanged().collect { (ready, atBottom, latest) ->
            if (ready) {
                // 非手势的插入和行变高不能被当成主人翻走；真正的滚动在交互观察里报告。
                if (atBottom) {
                    following = true
                    onAction(Action.SetTimelineScroll(roomId, TimelineScroll.BOTTOM))
                    if (latest != null) onAction(Action.MarkRoomRead(roomId))
                }
            }
        }
    }
    LaunchedEffect(roomId) {
        snapshotFlow { listState.isScrollInProgress to listState.isAtBottom() }
            .distinctUntilChanged().collect { (scrolling, atBottom) ->
                if (placed && scrolling && !atBottom) {
                    following = false
                    onAction(Action.SetTimelineScroll(roomId, TimelineScroll.AWAY))
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
            onAction(Action.SetTimelineScroll(roomId, TimelineScroll.AWAY))
        }
    }
    return atLatest || anchor == newest
}
/** 停在别处时的“回到底部”：数字是离开底部后到的消息数（ADR-0071），没有就不写数字。 */
@Composable
internal fun ScrollToBottomButton(
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
                scope.launch {
                    listState.animateScrollToItem(0)
                    onAction(Action.SetTimelineScroll(roomId, TimelineScroll.BOTTOM))
                    onAction(Action.MarkRoomRead(roomId))
                }
            },
            modifier = Modifier
                .size(WontonTheme.size.floatingActionButton)
                .semantics { contentDescription = label },
            containerColor = WontonTheme.colors.accentFill.toColor(),
            contentColor = WontonTheme.colors.accentInkOnFill.toColor(),
        ) {
            // 角标叠在圆钮外面：FAB 的 Surface 会按圆形裁剪内容，放进 BadgedBox 超出的部分会被切掉。
            Box {
                // 装饰：圆钮外层已经带了名字。
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

/** 第 0 项是最新的一条，贴着底边、没滑出去才算停在底部。 */
private fun LazyListState.isAtBottom(): Boolean =
    firstVisibleItemIndex == 0 && firstVisibleItemScrollOffset == 0

/** 最早的一条露出来了，该向前翻页。 */
internal fun LazyListState.reachedTop(timeline: TimelineState): Boolean {
    val last = layoutInfo.visibleItemsInfo.lastOrNull()?.index
    return timeline.loaded && last != null && last >= layoutInfo.totalItemsCount - 1 &&
        timeline.backPagination == BackPagination.IDLE
}
