package app.wonton.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import kotlin.math.abs
import kotlinx.coroutines.withTimeoutOrNull

/** 查看器的手势：双指缩放、放大后拖动、没放大时跟手下滑，两下没动的点击算双击。没放大时横向拖动不消费，留给翻页。 */
internal suspend fun PointerInputScope.viewerGestures(transform: ViewerTransform, onDismiss: () -> Unit) {
    awaitEachGesture {
        val firstDown = awaitFirstDown(requireUnconsumed = false)
        if (!trackGesture(transform, onDismiss)) return@awaitEachGesture
        // 第一次是点击：在双击时限内等第二下，两下都没动才算双击。
        val secondDown = withTimeoutOrNull(viewConfiguration.doubleTapTimeoutMillis) {
            awaitFirstDown(requireUnconsumed = false)
        } ?: return@awaitEachGesture
        val slop = viewConfiguration.touchSlop * DOUBLE_TAP_SLOP_IN_TOUCH_SLOP
        val secondWasTap = trackGesture(transform, onDismiss)
        if (secondWasTap && (secondDown.position - firstDown.position).getDistance() <= slop) {
            transform.toggleZoom(secondDown.position)
        }
    }
}

/**
 * 一次手势从按下到抬起，每一轮事件分给 [DragTracker] 的对应分支；松手后收尾。返回它是不是一次点击（没动过）。
 */
private suspend fun AwaitPointerEventScope.trackGesture(
    transform: ViewerTransform,
    onDismiss: () -> Unit,
): Boolean {
    val tracker = DragTracker(viewConfiguration.touchSlop)
    while (true) {
        val event = awaitPointerEvent()
        val pressed = event.changes.filter { it.pressed }
        if (pressed.isEmpty()) break
        when {
            pressed.size >= 2 -> tracker.pinch(event, transform)
            transform.scale > 1f -> tracker.pan(event, transform)
            else -> tracker.drag(event, transform)
        }
    }
    if (tracker.zooming && transform.scale <= 1f) transform.settle()
    if (!tracker.zooming) transform.settleDismiss(tracker.dismissed, onDismiss)
    return tracker.isTap
}

/**
 * 一次按下到抬起里攒下的手势状态：是不是点击、有没有在缩放、跟手关闭走到哪。
 * 消费规则：缩放和放大后的拖动全消费（翻页接不了手）；没放大时只在认领了下滑之后消费，横向拖动留给 HorizontalPager。
 */
private class DragTracker(private val touchSlop: Float) {
    var isTap = true
        private set
    var zooming = false
        private set
    var dismissed = Offset.Zero
        private set
    private var closing = false
    private var pending = Offset.Zero

    /** 双指缩放：跟手，翻页接不了手。 */
    fun pinch(event: PointerEvent, transform: ViewerTransform) {
        if (!zooming) {
            zooming = true
            isTap = false
            dismissed = Offset.Zero
            pending = Offset.Zero
            transform.applyDismiss(Offset.Zero)
        }
        transform.applyPinch(event)
        event.changes.forEach { it.consume() }
    }

    /** 放大后的拖动：由缩放接管。放大状态下没动过的点击仍是 tap，双击还原才认得出来。 */
    fun pan(event: PointerEvent, transform: ViewerTransform) {
        if (event.calculatePan() != Offset.Zero) isTap = false
        transform.applyPan(event)
        event.changes.forEach { it.consume() }
    }

    /** 没放大：拖过容差按方向挑一边。往下认作关闭，从认领那刻起跟手；横向不消费，翻页接手后这轮就不再跟。 */
    fun drag(event: PointerEvent, transform: ViewerTransform) {
        val drag = event.calculatePan()
        if (drag != Offset.Zero) isTap = false
        if (closing) {
            dismissed += drag
            transform.applyDismiss(dismissed)
            event.changes.forEach { it.consume() }
        } else if (event.changes.none { it.isConsumed }) {
            pending += drag
            if (pending.getDistance() > touchSlop && abs(pending.y) > abs(pending.x)) {
                closing = true
                dismissed = Offset.Zero
                transform.applyDismiss(Offset.Zero)
                event.changes.forEach { it.consume() }
            }
        }
    }
}
