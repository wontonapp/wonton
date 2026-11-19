package app.wonton.ui

import androidx.compose.foundation.shape.GenericShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalDensity
import app.wonton.ui.design.WontonTheme

/**
 * 气泡的轮廓：贴近发送者一侧的角在组内收小，组首的上角、组尾的下角保持大圆角。组尾在发送者一侧带一条尾巴，
 * 尾巴占 [WontonTheme.size.bubbleTail] 的宽度，形状把这段宽度让出来。
 */
@Composable
internal fun bubbleShape(isOwn: Boolean, joinsPrevious: Boolean, joinsNext: Boolean): Shape {
    val full = WontonTheme.radii.bubble
    val joined = WontonTheme.radii.bubbleJoined
    val tail = WontonTheme.size.bubbleTail
    val tailed = !joinsNext
    val density = LocalDensity.current
    return remember(isOwn, joinsPrevious, joinsNext, density, full, joined, tail) {
        val tailPx = with(density) { tail.toPx() }
        val fullPx = with(density) { full.toPx() }
        // 贴着发送者那一侧：组内收窄到 [joined]，组尾那个角留给尾巴。
        val senderPx = with(density) { (if (joinsPrevious) joined else full).toPx() }
        val corner = if (tailed) 0f else senderPx
        val (start, end) = if (isOwn) senderPx to fullPx else fullPx to senderPx
        GenericShape { size, _ ->
            val tailWidth = if (tailed) tailPx else 0f
            val left = if (isOwn) 0f else tailWidth
            val right = size.width - if (isOwn) tailWidth else 0f
            addRoundRect(
                RoundRect(
                    Rect(left, 0f, right, size.height),
                    topLeft = CornerRadius(if (isOwn) fullPx else start),
                    topRight = CornerRadius(start),
                    bottomRight = CornerRadius(if (isOwn) corner else end),
                    bottomLeft = CornerRadius(if (isOwn) end else corner),
                ),
            )
            if (tailed) tailPath(isOwn, left, right, size.height, tailPx)
        }
    }
}


/** 尾巴接在角上：自己的在右下，别人的在左下，和圆角一样指向对方。 */
private fun Path.tailPath(isOwn: Boolean, left: Float, right: Float, bottom: Float, tail: Float) {
    val base = if (isOwn) right else left
    val tip = if (isOwn) right + tail else left - tail
    moveTo(base, bottom - tail)
    lineTo(tip, bottom)
    lineTo(base, bottom)
    close()
}
