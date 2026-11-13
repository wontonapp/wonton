package app.wonton.ui

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val INNER_EDGE_FADE_FRACTION = 0.25f

/** Only the timeline is recorded. Replaying its display list never composes messages twice. */
internal class RoomBackdrop(val layer: GraphicsLayer) {
    var origin by mutableStateOf(Offset.Zero)
}

internal fun Modifier.recordRoomBackdrop(backdrop: RoomBackdrop): Modifier =
    onGloballyPositioned { backdrop.origin = it.positionInRoot() }.drawWithContent {
        backdrop.layer.record { this@drawWithContent.drawContent() }
        drawLayer(backdrop.layer)
    }

@Composable
internal fun Modifier.roomBackdrop(backdrop: RoomBackdrop, top: Boolean): Modifier {
    val replay = rememberGraphicsLayer()
    var origin by remember { mutableStateOf(Offset.Zero) }
    val radius = with(LocalDensity.current) { WontonTheme.size.menuBlur.toPx() }
    val canvas = WontonTheme.colors.canvas.toColor()
    // API 29/30 have no RenderEffect: retain the readable translucent gradient, not simulated blur.
    val supportsBlur = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val strong = canvas.copy(alpha = if (supportsBlur) 0.90f else 0.96f)
    val weak = canvas.copy(alpha = if (supportsBlur) 0.64f else 0.85f)
    val gradient = Brush.verticalGradient(if (top) listOf(strong, weak) else listOf(weak, strong))
    val effect = remember(radius) { BlurEffect(radius, radius, TileMode.Clamp) }
    val materialPaint = remember { Paint() }
    val edgeMask = remember(top) {
        if (top) {
            Brush.verticalGradient(
                0f to Color.Black,
                (1f - INNER_EDGE_FADE_FRACTION) to Color.Black,
                1f to Color.Transparent,
            )
        } else {
            Brush.verticalGradient(
                0f to Color.Transparent,
                INNER_EDGE_FADE_FRACTION to Color.Black,
                1f to Color.Black,
            )
        }
    }
    return onGloballyPositioned { origin = it.positionInRoot() }.drawWithContent {
        // Mask the replay and tint together; drawing foreground after restore keeps controls sharp.
        val drawCanvas = drawContext.canvas
        drawCanvas.saveLayer(Rect(Offset.Zero, size), materialPaint)
        if (supportsBlur) {
            replay.renderEffect = effect
            replay.record {
                val offset = backdrop.origin - origin
                translate(offset.x, offset.y) { drawLayer(backdrop.layer) }
            }
            clipRect { drawLayer(replay) }
        }
        drawRect(gradient)
        drawRect(edgeMask, blendMode = BlendMode.DstIn)
        drawCanvas.restore()
        drawContent()
    }
}

@Composable
internal fun RoomSurface(
    bar: @Composable () -> Unit,
    content: @Composable (RoomBackdrop) -> Unit,
) {
    val layer = rememberGraphicsLayer()
    val backdrop = remember(layer) { RoomBackdrop(layer) }
    val density = LocalDensity.current
    var barHeight by remember { mutableIntStateOf(0) }
    Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
        CompositionLocalProvider(LocalWontonTopInset provides with(density) { barHeight.toDp() }) {
            content(backdrop)
        }
        Box(Modifier.roomBackdrop(backdrop, top = true).onSizeChanged { barHeight = it.height }) { bar() }
    }
}
