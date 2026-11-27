package app.wonton.ui.photos

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.lerp
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.center
import androidx.compose.ui.unit.toOffset
import androidx.compose.ui.util.lerp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlin.math.abs

private const val MAX_SCALE = 5f
private const val DOUBLE_TAP_SCALE = 2.5f
private const val SETTLE_MS = 200

/** 大图页一张图的缩放和位移（绕屏幕中心），外加没放大时跟手下滑的距离。 */
internal class PhotoZoom(private val scope: CoroutineScope) {
    var scale by mutableFloatStateOf(1f)
        private set
    var offset by mutableStateOf(Offset.Zero)
        private set

    /** 没放大时手指往下拖了多远；松手够远就关大图页。 */
    var dismiss by mutableFloatStateOf(0f)
        private set
    var viewport = IntSize.Zero
    private var job: Job? = null

    val zoomed: Boolean get() = scale > 1f

    /** 两指：缩放时两指中点底下的那一点留在原处。 */
    fun pinch(event: PointerEvent) {
        job?.cancel()
        val next = (scale * event.calculateZoom()).coerceIn(1f, MAX_SCALE)
        val pivot = event.calculateCentroid() - viewport.center.toOffset()
        offset = clamped((offset - pivot) * (next / scale) + pivot + event.calculatePan(), next)
        scale = next
    }

    fun pan(delta: Offset) {
        job?.cancel()
        offset = clamped(offset + delta, scale)
    }

    fun drag(distance: Float) {
        job?.cancel()
        dismiss = distance.coerceAtLeast(0f)
    }

    /** 双击：没放大就把点的那一处放大，放大着就还原。 */
    fun toggle(tap: Offset) {
        val target = if (zoomed) 1f else DOUBLE_TAP_SCALE
        val pivot = tap - viewport.center.toOffset()
        animate(target, if (target == 1f) Offset.Zero else clamped(pivot * (1f - target), target), 0f)
    }

    /** 回到一比一、不偏移：下滑没到关的距离时弹回，翻到别的页时还原。 */
    fun reset() = animate(1f, Offset.Zero, 0f)

    private fun animate(toScale: Float, toOffset: Offset, toDismiss: Float) {
        job?.cancel()
        val fromScale = scale
        val fromOffset = offset
        val fromDismiss = dismiss
        job = scope.launch {
            Animatable(0f).animateTo(1f, tween(SETTLE_MS)) {
                scale = lerp(fromScale, toScale, value)
                offset = lerp(fromOffset, toOffset, value)
                dismiss = lerp(fromDismiss, toDismiss, value)
            }
        }
    }

    /** 放大后的图不拖出屏幕：偏移最多是放大多出来的那一半。 */
    private fun clamped(target: Offset, at: Float): Offset {
        val x = (at - 1f) * viewport.width / 2f
        val y = (at - 1f) * viewport.height / 2f
        return Offset(target.x.coerceIn(-x, x), target.y.coerceIn(-y, y))
    }
}

private enum class ZoomGesture { UNDECIDED, PINCH, PAN, DISMISS, PAGER }

/** 一次手势认成了什么：两指缩放、放大后拖图、没放大时往下拖（关闭），还是横着拖（留给翻页）。 */
private class ZoomTracker(private val zoom: PhotoZoom, private val slop: Float) {
    var gesture = ZoomGesture.UNDECIDED
        private set
    private var total = Offset.Zero

    /** 认下来归这里的手势要把事件吃掉，外面的翻页才不跟；没认下来和留给翻页的不吃。 */
    val owns: Boolean get() = gesture != ZoomGesture.UNDECIDED && gesture != ZoomGesture.PAGER

    fun track(event: PointerEvent) {
        val pressed = event.changes.count { it.pressed }
        when {
            pressed >= 2 -> {
                gesture = ZoomGesture.PINCH
                zoom.pinch(event)
            }
            pressed == 0 || gesture == ZoomGesture.PAGER || gesture == ZoomGesture.PINCH && !zoom.zoomed -> Unit
            zoom.zoomed -> {
                gesture = ZoomGesture.PAN
                zoom.pan(event.calculatePan())
            }
            else -> follow(event.calculatePan())
        }
    }

    /** 没放大时的单指：过了触摸 slop 才定方向，往下归关闭，其余留给翻页。 */
    private fun follow(pan: Offset) {
        total += pan
        if (gesture == ZoomGesture.UNDECIDED && total.getDistance() > slop) {
            gesture = if (total.y > abs(total.x)) ZoomGesture.DISMISS else ZoomGesture.PAGER
        }
        if (gesture == ZoomGesture.DISMISS) zoom.drag(total.y)
    }
}

/**
 * 大图页一张图上的拖动手势：两指缩放，放大后单指拖图；没放大时竖着往下拖是关闭，横着拖不吃、留给翻页。
 * 点和双击另由点击手势认。[closeDistance] 是下滑多远松手就关。
 */
internal suspend fun PointerInputScope.photoZoomGestures(zoom: PhotoZoom, closeDistance: Float, onClose: () -> Unit) {
    awaitEachGesture {
        awaitFirstDown(requireUnconsumed = false)
        val tracker = ZoomTracker(zoom, viewConfiguration.touchSlop)
        do {
            val event = awaitPointerEvent()
            tracker.track(event)
            if (tracker.owns) event.changes.forEach { it.consume() }
        } while (event.changes.any { it.pressed })
        if (tracker.gesture == ZoomGesture.DISMISS) {
            if (zoom.dismiss > closeDistance) onClose() else zoom.reset()
        }
    }
}
