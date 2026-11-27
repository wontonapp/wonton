package app.wonton.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange

/**
 * 行上按下后横着拖过触摸阈值，就把这次按压从“点按”里摘掉：拖动不再当成点开这一行（#931 的商店行）。
 *
 * 和 `clickable` 同一处用、放在它后面：这个节点更靠里，先看到移动并吃掉，`clickable` 随即把按压判成取消。
 * 先竖着走的不拦，列表照常滚。
 */
internal fun Modifier.cancelClickOnDrag(): Modifier = pointerInput(Unit) {
    awaitEachGesture {
        val down = awaitFirstDown(requireUnconsumed = false)
        var dx = 0f
        var dy = 0f
        var axis = DragAxis.Undecided
        var alive = true
        while (alive) {
            val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
            if (change == null || !change.pressed) {
                alive = false
            } else if (axis == DragAxis.Horizontal) {
                change.consume()
            } else if (axis == DragAxis.Undecided) {
                dx += change.positionChange().x
                dy += change.positionChange().y
                axis = axisAfter(dx, dy, viewConfiguration.touchSlop)
                if (axis == DragAxis.Horizontal) change.consume()
            }
        }
    }
}
