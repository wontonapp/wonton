package app.wonton.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Reply
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch
import kotlinx.coroutines.Job

/** 右滑多远算回复：过了这里松手就进回复状态，也在这里给一次触感。 */
private val THRESHOLD = 40.dp

/** 跟着手指最多移多远。 */
private val MAX = 72.dp

/**
 * 右滑回复（ADR-0073 第 3 节）：只有菜单里有 `Reply` 的消息滑得动，过阈值松手进回复。
 * 气泡跟着手指挪，[offset] 给气泡画位移，[modifier] 挂在气泡那一层上。
 */
class SwipeToReplyState(
    val enabled: Boolean,
    private val haptics: HapticFeedback,
    private val onReply: () -> Unit,
    private val scope: kotlinx.coroutines.CoroutineScope,
    private val threshold: Float,
    private val max: Float,
) {
    val offset = mutableFloatStateOf(0f)
    private var armed = false
    private var crossedThreshold = false
    private var settleJob: Job? = null

    val modifier: Modifier = if (!enabled) {
        Modifier
    } else {
        Modifier
            .graphicsLayer { translationX = offset.value }
            .pointerInput(Unit) {
                detectHorizontalDragGestures(
                    onDragStart = {
                        settleJob?.cancel()
                        armed = false
                        crossedThreshold = false
                    },
                    onDragEnd = { finish(armed) },
                    onDragCancel = { finish(false) },
                    onHorizontalDrag = { change, drag ->
                        if (offset.floatValue > 0f || drag > 0f) change.consume()
                        offset.floatValue = (offset.floatValue + drag).coerceIn(0f, max)
                        armed = offset.floatValue >= threshold
                        if (armed && !crossedThreshold) {
                            crossedThreshold = true
                            haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                        }
                    },
                )
            }
    }

    /** A reply never leaves the row open, including a canceled or below-threshold drag. */
    private fun finish(reply: Boolean) {
        settleJob?.cancel()
        if (reply) onReply()
        settleJob = scope.launch {
            Animatable(offset.floatValue).animateTo(0f, spring()) { offset.floatValue = value }
        }
    }
}

@Composable
fun rememberSwipeToReply(enabled: Boolean, onReply: () -> Unit): SwipeToReplyState {
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val current by rememberUpdatedState(onReply)
    return remember(enabled, haptics, scope, density) {
        SwipeToReplyState(
            enabled = enabled,
            haptics = haptics,
            onReply = { current() },
            scope = scope,
            threshold = with(density) { THRESHOLD.toPx() },
            max = with(density) { MAX.toPx() },
        )
    }
}

/** 右滑时露在前面的箭头，越靠近阈值越清楚。位置跟着气泡走。 */
@Composable
fun SwipeHint(swipe: SwipeToReplyState) {
    val offset = swipe.offset.value
    if (!swipe.enabled || offset <= 0f) return
    val icon = with(LocalDensity.current) { WontonTheme.size.iconSmall.toPx() }
    val gap = with(LocalDensity.current) { WontonTheme.spacing.s8.toPx() }
    Icon(
        WontonIcon.Reply,
        // 装饰：跟手的临时箭头，松手就看结果。
        contentDescription = null,
        modifier = Modifier
            .padding(start = WontonTheme.spacing.s8)
            .graphicsLayer { translationX = offset - icon - gap }
            .size(WontonTheme.size.iconSmall)
            .alpha((offset / icon).coerceIn(0f, 1f)),
        tint = WontonTheme.colors.textSecondary.toColor(),
    )
}
