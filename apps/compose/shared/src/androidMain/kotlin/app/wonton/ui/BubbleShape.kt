package app.wonton.ui

import androidx.compose.foundation.shape.GenericShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import app.wonton.ui.design.WontonTheme

/**
 * 气泡的轮廓：贴近发送者一侧的角在组内收小，组首的上角、组尾的下角保持大圆角。组尾在发送者一侧带一条尾巴，
 * 尾巴占 [WontonTheme.size.bubbleTail] 的宽度；组内没有尾巴的也让出这段宽度，一组里的气泡起点才对得齐。
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
            val left = if (isOwn) 0f else tailPx
            val right = size.width - if (isOwn) tailPx else 0f
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

/** 有选项的那条按内容撑开到按钮行那么宽。 */
internal fun columnWrap(hasChoices: Boolean) = if (hasChoices) Modifier.width(IntrinsicSize.Max) else Modifier

/** 自己的消息靠右，别人的靠左。 */
internal fun rowArrangement(isOwn: Boolean) =
    if (isOwn) {
        Arrangement.spacedBy(WontonTheme.spacing.s4, Alignment.End)
    } else {
        Arrangement.spacedBy(WontonTheme.spacing.s4, Alignment.Start)
    }

/** 有选项时气泡跟着整列走满，按钮行才不会比气泡宽；其余情况继续限宽。 */
internal fun bubbleWrap(hasChoices: Boolean, bubbleWidth: Dp) =
    if (hasChoices) Modifier else Modifier.widthIn(max = bubbleWidth)
