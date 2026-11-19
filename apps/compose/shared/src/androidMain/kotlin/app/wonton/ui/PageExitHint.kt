package app.wonton.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonTheme

/** 手指滑过的距离和一趟的时长：够看清方向就行。 */
private val SWIPE_TRAVEL = 120.dp
private val FINGER = 44.dp
private const val SWIPE_MS = 1400
/** 一趟里最后这一段手指淡出，再从边缘重新出现，读成“滑完抬手”。 */
private const val LIFT_FROM = 0.75f
private const val TRACK_ALPHA = 0.25f
private val TEXT_MAX = 320.dp

/**
 * 第一次打开整页应用时的引导：应用页上没有任何壳的按钮（ADR-0095），出口只有系统返回，不说的话找不到。
 * 遮住整页并吃掉触摸，点了“知道了”才记住；用返回离开不算看过，下次打开还教。
 */
@Composable
internal fun PageExitHint() {
    val context = LocalContext.current
    val prefs = remember(context) { PageExitHintPrefs.of(context) }
    if (prefs.seen) return
    AvatarEnvironment.observeSystem(context)
    Box(
        Modifier.zIndex(2f).fillMaxSize()
            .background(Color.Black.copy(alpha = WontonTheme.ratio.scrim))
            .pointerInput(Unit) { detectTapGestures { } }
            .testTag("page-exit-hint"),
    ) {
        Column(
            Modifier.align(Alignment.Center).fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s24),
        ) {
            Box(Modifier.align(Alignment.Start)) { SwipeFromEdge(animated = !AvatarEnvironment.reduceMotion) }
            Text(
                stringResource(R.string.tab_exit_hint),
                Modifier.widthIn(max = TEXT_MAX).padding(horizontal = WontonTheme.spacing.s32),
                style = WontonTheme.typography.itemTitle, color = Color.White, textAlign = TextAlign.Center,
            )
            WontonButton(stringResource(R.string.tab_exit_hint_got_it), { prefs.seen = true })
        }
    }
}

/** 从左边缘往里滑的手指：一条淡的轨迹，上面一个圆点。装饰，不进读屏（那句话已经说了）。减弱动态效果时停在滑完的位置。 */
@Composable
private fun SwipeFromEdge(animated: Boolean) {
    val progress = if (animated) {
        val transition = rememberInfiniteTransition(label = "page-exit-hint")
        val value by transition.animateFloat(
            0f, 1f, infiniteRepeatable(tween(SWIPE_MS, easing = LinearEasing), RepeatMode.Restart), label = "swipe",
        )
        value
    } else {
        LIFT_FROM
    }
    val slide = (progress / LIFT_FROM).coerceAtMost(1f)
    val lift = if (progress > LIFT_FROM) 1f - (progress - LIFT_FROM) / (1f - LIFT_FROM) else 1f
    Box(
        Modifier.size(SWIPE_TRAVEL + FINGER, FINGER)
            .background(
                Color.White.copy(alpha = TRACK_ALPHA),
                RoundedCornerShape(topEnd = FINGER / 2, bottomEnd = FINGER / 2),
            ),
    ) {
        Box(Modifier.offset(x = travel(slide)).size(FINGER).alpha(lift).background(Color.White, CircleShape))
    }
}

private fun travel(fraction: Float): Dp = SWIPE_TRAVEL * fraction
