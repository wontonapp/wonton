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
import kotlinx.coroutines.flow.drop
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
