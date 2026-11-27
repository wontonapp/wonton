package app.wonton.ui

import android.content.ContentResolver
import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.systemGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Density
import app.wonton.ui.design.WontonTheme
import kotlin.math.abs

/** 系统返回手势区按它缩放；用户把返回调灵敏时，系统区比 24dp 还窄。 */
private const val BACK_GESTURE_INSET_SCALE_LEFT = "back_gesture_inset_scale_left"

/**
 * 左边缘 24dp 内起手的右滑 = 返回（#931 的 Android 半）。
 *
 * 系统自己的返回手势只认它贴边那一条，那条的宽窄跟着返回灵敏度缩放：调灵敏以后 24dp 里剩下的那截没人接，
 * 从那里起手拖动会被列表行当成点按。这里只补系统区以外、24dp 以内那一截，和系统那条错开、不重叠，
 * 不会退两次返回；结束时交给返回分发器，关面板、关页、退 App 都由已有的处理器决定。
 *
 * 包在内容外面而不是盖在上面：手指在这一截里点一下，行和按钮照常收得到；横着拖才吃掉移动，
 * 把这一下从点按里摘掉。先竖着走的原样留给列表。
 */
@Composable
internal fun EdgeBackSwipe(
    enabled: Boolean,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit,
) {
    val density = LocalDensity.current
    val densityValue = density.density
    val back by rememberUpdatedState(onBack)
    val startPx = systemGestureAreaPx(density) * backGestureInsetScale()
    val widthPx = if (enabled) edgeBackWidthPx(WontonTheme.gesture.edgeWidth, startPx, densityValue) else 0f
    val swipe = if (widthPx <= 0f) Modifier else Modifier.edgeBackSwipe(startPx, startPx + widthPx, densityValue, back)
    Box(modifier.then(swipe).testTag("edge-back"), content = content)
}

/** 起手点落在 `[fromPx, untilPx)` 才管；横着走过触摸阈值就吃掉移动，松手够远够快才返回。 */
private fun Modifier.edgeBackSwipe(fromPx: Float, untilPx: Float, density: Float, onBack: () -> Unit): Modifier =
    pointerInput(fromPx, untilPx, density) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            if (down.position.x < fromPx || down.position.x >= untilPx) return@awaitEachGesture
            val tracker = VelocityTracker()
            tracker.addPosition(down.uptimeMillis, down.position)
            var travel = 0f
            var axis = DragAxis.Undecided
            var alive = true
            while (alive) {
                val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
                if (change == null || !change.pressed) {
                    alive = false
                } else {
                    tracker.addPosition(change.uptimeMillis, change.position)
                    travel = change.position.x - down.position.x
                    val dy = change.position.y - down.position.y
                    axis = when (axis) {
                        // 位移按离起手点的绝对值算：外面谁先吃掉了移动，这里也照样看得见手指走了多远。
                        DragAxis.Undecided -> axisAfter(travel, dy, viewConfiguration.touchSlop)
                        else -> axis
                    }
                    if (axis == DragAxis.Horizontal) change.consume()
                }
            }
            if (edgeBackCommits(axis, travel, tracker.calculateVelocity().x, density)) onBack()
        }
    }

/** 横着走下来的、松手够远或够快才算一次返回。 */
private fun edgeBackCommits(axis: DragAxis, travel: Float, velocity: Float, density: Float): Boolean =
    axis == DragAxis.Horizontal &&
        commitsGesture(travel, velocity, WontonTheme.gesture.swipeDistance, density)

/** 起手区宽度（px）：左边缘 24dp 里去掉系统自己那条，系统比 24dp 还宽就不留。 */
internal fun edgeBackWidthPx(edgeWidthDp: Float, systemAreaPx: Float, density: Float): Float =
    (edgeWidthDp * density - systemAreaPx).coerceAtLeast(0f)

/** 系统返回手势区的左宽（px）；29 上没有这个读取，按 0 算，整条 24dp 都归我们。 */
@Composable
private fun systemGestureAreaPx(density: Density): Float = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
    WindowInsets.systemGestures.getLeft(density, LocalLayoutDirection.current).toFloat()
} else {
    0f
}

/** 返回灵敏度的缩放；系统报的系统区宽度不跟它变，得自己乘。改设置时跟着更新。 */
@Composable
private fun backGestureInsetScale(): Float {
    val resolver = LocalContext.current.contentResolver
    var scale by remember { mutableFloatStateOf(resolver.backGestureScale()) }
    DisposableEffect(resolver) {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                scale = resolver.backGestureScale()
            }
        }
        resolver.registerContentObserver(Settings.Secure.getUriFor(BACK_GESTURE_INSET_SCALE_LEFT), false, observer)
        onDispose { resolver.unregisterContentObserver(observer) }
    }
    return scale
}

private fun ContentResolver.backGestureScale(): Float =
    Settings.Secure.getFloat(this, BACK_GESTURE_INSET_SCALE_LEFT, 1f)
