package app.wonton.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.lerp
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.util.lerp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

private const val MAX_SCALE = 5f
private const val DOUBLE_TAP_SCALE = 2.5f
private const val ZOOM_MS = 220

internal const val DOUBLE_TAP_SLOP_IN_TOUCH_SLOP = 4f

internal class ViewerTransform(
    private val scope: CoroutineScope,
    private val closeThresholdPx: Float,
) {
    var scale by mutableFloatStateOf(1f)
        private set
    var offset by mutableStateOf(Offset.Zero)
        private set
    var dismiss by mutableStateOf(Offset.Zero)
        private set
    var viewport = IntSize.Zero
    private var zoomJob: Job? = null
    private var dismissJob: Job? = null

    fun applyPinch(event: PointerEvent) {
        val zoomChange = event.calculateZoom()
        val panChange = event.calculatePan()
        if (zoomChange == 1f && panChange == Offset.Zero) return
        val newScale = (scale * zoomChange).coerceIn(1f, MAX_SCALE)
        val centroid = event.calculateCentroid()
        val target = (offset - centroid) * (newScale / scale) + centroid + panChange
        setZoom(newScale, clamped(target, newScale))
    }

    fun applyPan(event: PointerEvent) {
        val panChange = event.calculatePan()
        if (panChange == Offset.Zero) return
        setZoom(scale, clamped(offset + panChange, scale))
    }

    fun applyDismiss(drag: Offset) {
        dismissJob?.cancel()
        dismiss = drag
    }

    fun toggleZoom(centroid: Offset) {
        val targetScale = if (scale > 1f) 1f else DOUBLE_TAP_SCALE
        val targetOffset = if (targetScale == 1f) Offset.Zero
            else clamped(centroid * (1f - targetScale), targetScale)
        animateZoom(targetScale, targetOffset)
    }

    fun settle() = animateZoom(1f, Offset.Zero)

    fun settleDismiss(dismissed: Offset, onDismiss: () -> Unit) {
        if (dismissed.y > closeThresholdPx) {
            onDismiss()
        } else {
            dismissJob?.cancel()
            val from = dismiss
            dismissJob = scope.launch {
                Animatable(0f).animateTo(1f, spring(stiffness = Spring.StiffnessMediumLow)) {
                    dismiss = lerp(from, Offset.Zero, value)
                }
            }
        }
    }

    private fun setZoom(newScale: Float, newOffset: Offset) {
        zoomJob?.cancel()
        scale = newScale
        offset = newOffset
    }

    private fun animateZoom(targetScale: Float, targetOffset: Offset) {
        zoomJob?.cancel()
        val fromScale = scale
        val fromOffset = offset
        zoomJob = scope.launch {
            Animatable(0f).animateTo(1f, tween(ZOOM_MS)) {
                scale = lerp(fromScale, targetScale, value)
                offset = lerp(fromOffset, targetOffset, value)
            }
        }
    }

    private fun clamped(target: Offset, scaleValue: Float): Offset {
        val bound = Offset(
            (scaleValue - 1f) * viewport.width / 2f,
            (scaleValue - 1f) * viewport.height / 2f,
        )
        return Offset(target.x.coerceIn(-bound.x, bound.x), target.y.coerceIn(-bound.y, bound.y))
    }
}
