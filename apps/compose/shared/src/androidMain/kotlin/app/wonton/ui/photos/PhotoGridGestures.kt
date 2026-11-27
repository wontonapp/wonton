package app.wonton.ui.photos

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitLongPressOrCancellation
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

/** 两指张开到这个倍数格子变大一档，捏到它的倒数变小一档。 */
private const val PINCH_STEP = 1.25f

/** 连选时手指离上下边这么近就自己滚。 */
private val AUTO_SCROLL_ZONE = 72.dp
private const val AUTO_SCROLL_PX_PER_FRAME = 28f
private const val FRAME_MS = 16L

/**
 * 捏合换列数：两指落下后归这里，子项（滚动、点击）收不到这次手势。[onStep] 给 -1 是放大一档（列数变少），
 * +1 是缩小一档。
 */
internal fun Modifier.pinchLevels(onStep: (Int) -> Unit): Modifier = pointerInput(Unit) {
    awaitEachGesture {
        awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
        var zoom = 1f
        do {
            val event = awaitPointerEvent(PointerEventPass.Initial)
            if (event.changes.count { it.pressed } >= 2) {
                zoom *= event.calculateZoom()
                event.changes.forEach { it.consume() }
                if (zoom > PINCH_STEP) {
                    onStep(-1)
                    zoom = 1f
                } else if (zoom < 1f / PINCH_STEP) {
                    onStep(1)
                    zoom = 1f
                }
            }
        } while (event.changes.any { it.pressed })
    }
}

/** 一次长按连选：起点那格、开始时已经选中的，和这次拖过的范围。 */
private class DragSelection(val anchor: Int, val before: Set<String>) {
    var range: Set<String> = emptySet()
}

/** 长按选中、接着拖动连选要的东西：网格里所有格子的先后、现在选中的，和把增减发回核心的出口。 */
internal class GridSelection(
    val ids: List<String>,
    val selected: List<String>,
    val onSelect: (ids: List<String>, selected: Boolean) -> Unit,
)

/**
 * 长按一格进入选择，不抬手接着拖就把起点到手指之间的一段都选上，往回拖的退掉（开始前就选中的不动）。
 * 长按成立以后的事件在子项之前吃掉：抬手时格子自己的点击不会再把刚选中的这格点掉。
 */
@Composable
internal fun Modifier.dragSelect(grid: LazyGridState, selection: GridSelection): Modifier {
    val latest by rememberUpdatedState(selection)
    val haptics = LocalHapticFeedback.current
    val zone = with(LocalDensity.current) { AUTO_SCROLL_ZONE.toPx() }
    var autoScroll by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(autoScroll) {
        while (autoScroll != 0f) {
            grid.scrollBy(autoScroll)
            delay(FRAME_MS)
        }
    }
    return pointerInput(grid) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val press = awaitLongPressOrCancellation(down.id)
            val start = press?.let { grid.tileAt(it.position) }?.let { latest.ids.indexOf(it) } ?: -1
            if (start >= 0) {
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                val drag = DragSelection(start, latest.selected.toSet())
                extend(drag, start, latest)
                followDrag(down.id) { position ->
                    autoScroll = when {
                        position.y < zone -> -AUTO_SCROLL_PX_PER_FRAME
                        position.y > size.height - zone -> AUTO_SCROLL_PX_PER_FRAME
                        else -> 0f
                    }
                    val at = grid.tileAt(position)?.let { latest.ids.indexOf(it) } ?: -1
                    if (at >= 0) extend(drag, at, latest)
                }
                autoScroll = 0f
            }
        }
    }
}

/** 长按成立以后跟着这根手指到抬手：事件在子项之前吃掉，每动一下把位置交给 [onMove]。 */
private suspend fun AwaitPointerEventScope.followDrag(pointer: PointerId, onMove: (Offset) -> Unit) {
    var pressed = true
    while (pressed) {
        val change = awaitPointerEvent(PointerEventPass.Initial).changes.firstOrNull { it.id == pointer }
        change?.consume()
        pressed = change?.pressed == true
        if (change != null && pressed) onMove(change.position)
    }
}

/** 把这次拖动的范围改到 [to]：新进范围的选上，退出范围又不是原来就选中的取消。 */
private fun extend(drag: DragSelection, to: Int, selection: GridSelection) {
    val range = selection.ids.subList(minOf(drag.anchor, to), maxOf(drag.anchor, to) + 1).toSet()
    val added = range - drag.range - drag.before
    val removed = drag.range - range - drag.before
    drag.range = range
    if (added.isNotEmpty()) selection.onSelect(added.toList(), true)
    if (removed.isNotEmpty()) selection.onSelect(removed.toList(), false)
}

/** 手指底下是哪一格（条目 id）；落在标题、空白上为空。项的偏移从内容区起算，要加回视口的起点。 */
private fun LazyGridState.tileAt(position: Offset): String? {
    val y = position.y + layoutInfo.viewportStartOffset
    return layoutInfo.visibleItemsInfo.firstOrNull { item ->
        position.x >= item.offset.x && position.x < item.offset.x + item.size.width &&
            y >= item.offset.y && y < item.offset.y + item.size.height
    }?.let { (it.key as? String)?.takeIf { key -> key.startsWith(GridEntry.TILE_KEY) } }
        ?.removePrefix(GridEntry.TILE_KEY)
}
