package app.wonton.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.util.VelocityTracker
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.math.abs

/** 点边区换一张：左右各占这么宽的一屏（比例），中间的点不动。 */
private const val PAGE_TAP_ZONE = 1f / 3f

/**
 * 查看器的手势：双指缩放、放大后拖动、没放大时跟手下滑关闭或多图时横移翻页，两下没动的点击算双击。
 * 双击时限内没等到第二下：点在左右边区就换一张（缩放时不换）。
 */
internal suspend fun PointerInputScope.viewerGestures(
    transform: ViewerTransform,
    pager: PageState?,
    onDismiss: () -> Unit,
) {
    awaitEachGesture {
        val firstDown = awaitFirstDown(requireUnconsumed = false)
        if (!trackGesture(transform, pager, onDismiss)) return@awaitEachGesture
        // 第一次是点击：在双击时限内等第二下，两下都没动才算双击。
        val secondDown = withTimeoutOrNull(viewConfiguration.doubleTapTimeoutMillis) {
            awaitFirstDown(requireUnconsumed = false)
        } ?: run {
            tapPage(transform, pager, firstDown.position)
            return@awaitEachGesture
        }
        val slop = viewConfiguration.touchSlop * DOUBLE_TAP_SLOP_IN_TOUCH_SLOP
        val secondWasTap = trackGesture(transform, pager, onDismiss)
        if (secondWasTap && (secondDown.position - firstDown.position).getDistance() <= slop) {
            transform.toggleZoom(secondDown.position)
        }
    }
}

/** 双击没等到：点在左右三分之一的边区换一张，中间不动；放大时点按不换页。 */
private fun tapPage(transform: ViewerTransform, pager: PageState?, down: Offset) {
    if (pager == null || transform.scale > 1f) return
    val width = transform.viewport.width.toFloat()
    if (width <= 0f) return
    val side = when {
        down.x < width * PAGE_TAP_ZONE -> -1
        down.x > width * (1f - PAGE_TAP_ZONE) -> 1
        else -> 0
    }
    if (side != 0) pager.tap(side)
}

/**
 * 一次手势从按下到抬起：跟手缩放、平移、下滑或横移翻页。返回它是不是一次点击（没动过、也没被子控件认领）。
 * 手势的累计状态都在 [ViewerGesture] 里，这里只管逐个事件分发。
 */
private suspend fun AwaitPointerEventScope.trackGesture(
    transform: ViewerTransform,
    pager: PageState?,
    onDismiss: () -> Unit,
): Boolean {
    val gesture = ViewerGesture()
    while (true) {
        val event = awaitPointerEvent()
        // 子控件（按钮）已经认领的手势让给它，查看器不再跟。按钮多在抬起那一刻才认领，
        // 所以要先看有没有被消费、再看手是不是都抬起了。
        if (event.changes.any { it.isConsumed }) {
            // 收尾照常（位移弹回/翻页落定），但这次不算点击：
            // 点在保存/分享上不该被当成点边区换页，也不该参与双击判定。
            gesture.finish(transform, pager, onDismiss)
            return false
        }
        val pressed = event.changes.filter { it.pressed }
        if (pressed.isEmpty()) break
        when {
            pressed.size >= 2 -> gesture.pinch(event, transform)
            transform.scale > 1f -> gesture.pan(event, transform)
            else -> gesture.follow(event, transform, pager, viewConfiguration.touchSlop.toFloat())
        }
        event.changes.forEach { it.consume() }
    }
    return gesture.finish(transform, pager, onDismiss)
}

/** 一次手势的累计状态：双指缩放、放大后平移、没放大时跟手（下滑关闭或横移翻页），抬手时各自落定。 */
private class ViewerGesture {
    var zooming = false
        private set
    var tap = true
        private set
    private var dismissed = Offset.Zero
    // 翻页锁：null 还没锁，true 横移翻页，false 竖滑关闭。
    private var paging: Boolean? = null
    private var handed = false
    private val tracker = VelocityTracker()

    fun pinch(event: PointerEvent, transform: ViewerTransform) {
        if (!zooming) {
            zooming = true
            tap = false
            dismissed = Offset.Zero
            transform.applyDismiss(Offset.Zero)
        }
        transform.applyPinch(event)
    }

    fun pan(event: PointerEvent, transform: ViewerTransform) {
        // 放大状态下没动过的点击仍是 tap，双击还原才认得出来。
        val drag = event.calculatePan()
        if (drag != Offset.Zero) tap = false
        transform.applyPan(event)
    }

    /** 没放大时的单指：先跟手，过触摸 slop 锁轴——横移归翻页（多图才有），竖滑归关闭。 */
    fun follow(event: PointerEvent, transform: ViewerTransform, pager: PageState?, slop: Float) {
        val drag = event.calculatePan()
        if (drag != Offset.Zero) tap = false
        tracker.addPosition(event.changes.first().uptimeMillis, event.changes.first().position)
        val total = dismissed + drag
        val overSlop = abs(total.x) > slop || abs(total.y) > slop
        if (pager != null && paging == null && overSlop) paging = abs(total.x) > abs(total.y)
        if (paging == true) {
            // 锁到横移那一刻，把之前跟手的位移整个交给翻页，关闭层归零不再跟。
            if (!handed) {
                pager?.applyDrag(total.x)
                handed = true
            } else {
                pager?.applyDrag(drag.x)
            }
            transform.applyDismiss(Offset.Zero)
        } else {
            dismissed = total
            transform.applyDismiss(dismissed)
        }
    }

    /** 抬手（或把事件让给子控件）时收尾：缩放复位、下滑关闭、翻页落定。返回是不是一次点击。 */
    fun finish(transform: ViewerTransform, pager: PageState?, onDismiss: () -> Unit): Boolean {
        if (zooming && transform.scale <= 1f) transform.settle()
        if (!zooming) {
            transform.settleDismiss(dismissed, onDismiss)
            // 横移过的手势松手：够远或够快翻到邻页，否则（含到头）弹回。
            if (paging == true) pager?.settle(tracker.calculateVelocity().x)
        }
        return tap
    }
}
