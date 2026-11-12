package app.wonton.ui

import android.content.res.Resources
import android.view.MotionEvent
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.runtime.Composable
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import androidx.compose.ui.unit.dp
import kotlin.math.abs

private val EDGE_WIDTH = 20.dp

/** 边缘滑动的距离换算成速度的量：一段 75px 的滑动够关掉面板。 */
private const val EDGE_SWIPE_GAIN = 10f

/** 边缘热区占屏高的比例：系统返回手势的排除区最多认 200dp，2400px 屏上 0.22 ≈ 528px（≈220dp，压到上限内）。 */
private const val EDGE_ZONE_RATIO = 0.22f
private const val SLOP_DP = 8f
@Composable
internal fun BoxScope.ChatPanelStrip(state: ChatPanelState) {
    val interaction = remember { MutableInteractionSource() }
    val label = stringResource(R.string.panel_close)
    val strip = with(LocalDensity.current) { state.strip.toDp() }
    Box(
        Modifier.fillMaxWidth()
            .height(strip)
            .align(if (state.edge == PanelEdge.TOP) Alignment.TopCenter else Alignment.BottomCenter)
            .clickable(interaction, indication = null) { state.move(PanelLevel.CLOSED) }
            .semantics { contentDescription = label }
            .pointerInput(state) {
                val tracker = VelocityTracker()
                detectDragGestures(
                    onDragStart = { state.begin() },
                    onDrag = { change, amount ->
                        change.consume()
                        tracker.addPosition(change.uptimeMillis, change.position)
                        state.pull(amount.y * state.sign)
                    },
                    onDragEnd = { state.settle(state.level, tracker.calculateVelocity().y * state.sign) },
                    onDragCancel = { state.settle(state.level, 0f) },
                )
            },
    ) {
        if (state.level == PanelLevel.PEEK) {
            Box(
                Modifier.align(if (state.edge == PanelEdge.TOP) Alignment.TopCenter else Alignment.BottomCenter)
                    .padding(vertical = WontonTheme.spacing.s8)
                    .size(WontonTheme.size.grabberWidth, WontonTheme.size.grabberHeight)
                    .background(WontonTheme.colors.separator.toColor(), CircleShape),
            )
        }
    }
}

/** 整页面板从顶部下拉回第 1 级；只占顶部一小段，不抢网页正文的手势。 */
@Composable
internal fun ChatPanelTopPull(state: ChatPanelState) {
    if (state.edge != PanelEdge.BOTTOM || state.level == PanelLevel.CLOSED) return
    val top = with(LocalDensity.current) { (state.height - state.shiftPx).toDp() }
    Box(
        Modifier.fillMaxWidth()
            .offset(y = top)
            .height(48.dp)
            .pointerInput(state) {
                val tracker = VelocityTracker()
                detectDragGestures(
                    onDragStart = { tracker.resetTracking() },
                    onDrag = { change, amount ->
                        change.consume()
                        tracker.addPosition(change.uptimeMillis, change.position)
                        state.pull(-amount.y)
                    },
                    onDragEnd = { state.settle(state.level, -tracker.calculateVelocity().y) },
                    onDragCancel = { state.settle(state.level, 0f) },
                )
            },
        contentAlignment = Alignment.TopCenter,
    ) {
        Box(
            Modifier.padding(vertical = WontonTheme.spacing.s8)
                .size(WontonTheme.size.grabberWidth, WontonTheme.size.grabberHeight)
                .background(WontonTheme.colors.separator.toColor(), CircleShape),
        )
    }
}

/** 左边缘往右滑关掉面板：面板开着时左边缘归面板（A 块），系统返回手势排除这一段。 */
@Suppress("CyclomaticComplexMethod") // 手势的每个分支都在这里。
@Composable
internal fun ChatPanelEdge(state: ChatPanelState, enabled: Boolean) {
    if (!enabled) return
    val label = stringResource(R.string.panel_close)
    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier.align(Alignment.CenterStart)
                .fillMaxHeight(EDGE_ZONE_RATIO)
                .width(EDGE_WIDTH)
                .systemGestureExclusion()
                .semantics { contentDescription = label }
                .pointerInput(state) {
                    var total = 0f
                    detectDragGestures(
                        onDragStart = { total = 0f },
                        onDrag = { change, amount ->
                            change.consume()
                            total += -amount.x
                            state.pull(-amount.x)
                        },
                        onDragEnd = { state.settle(state.level, total * EDGE_SWIPE_GAIN) },
                        onDragCancel = { state.settle(state.level, 0f) },
                    )
                },
        )
    }
}

/** 面板网页上的触摸：第 1 级左右拖翻页，主页面上滚到顶后继续往下拉进第 2 级；其余交给网页。
 *  实例按页面记住，回调由页面在每次重组时更新：拖动中途换实例会丢掉已经记下的手势。 */
internal class PanelPageTouch(private val state: ChatPanelState) {
    var atTop: () -> Boolean = { state.controller?.view?.scrollY?.let { it <= 0 } ?: true }
    var width: () -> Float = { 1f }

    /** 手指按下时的页位：翻页从这里算绝对位移，中途丢几个移动事件也不跟丢。 */
    var position: () -> Float = { 0f }

    /** 翻到绝对页位 [Float]，松手按速度落页。 */
    var flipTo: (Float) -> Unit = {}
    var flipEnd: (Float) -> Unit = {}
    private var flipBase = 0f

    /** 手指按下的屏幕坐标：页面自己会滑走，局部坐标跟着动，位移只在屏幕坐标里不变。 */
    private var downRawX = 0f
    private var downRawY = 0f

    /** 已经喂给面板和翻页的位移：移动事件被合并时，松手那一下把差值补上。 */
    private var pulled = 0f

    private var origin: Offset? = null
    private var last = Offset.Zero
    private var mode = Mode.NONE
    private val tracker = VelocityTracker()
    private val slop = SLOP_DP * Resources.getSystem().displayMetrics.density

    private enum class Mode { NONE, FLIP, PULL }

    fun observe(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                origin = Offset(event.x, event.y)
                downRawX = event.rawX
                downRawY = event.rawY
                last = origin ?: Offset.Zero
                mode = Mode.NONE
                pulled = 0f
                tracker.resetTracking()
                tracker.addPosition(event.eventTime, last)
            }
            MotionEvent.ACTION_MOVE -> move(event)
            MotionEvent.ACTION_UP -> {
                tracker.addPosition(event.eventTime, Offset(event.x, event.y))
                val velocity = tracker.calculateVelocity()
                catchUp(event)
                when (mode) {
                    Mode.FLIP -> flipEnd(velocity.x)
                    Mode.PULL -> state.settle(PanelLevel.PEEK, velocity.y)
                    Mode.NONE -> Unit
                }
                finish()
            }
            MotionEvent.ACTION_CANCEL -> {
                catchUp(event)
                when (mode) {
                    Mode.FLIP -> flipEnd(0f)
                    Mode.PULL -> state.settle(PanelLevel.PEEK, 0f)
                    Mode.NONE -> Unit
                }
                finish()
            }
        }
    }

    @Suppress("ReturnCount")
    private fun move(event: MotionEvent) {
        val start = origin ?: return
        val point = Offset(event.x, event.y)
        if (mode == Mode.NONE) {
            if (state.level != PanelLevel.PEEK) return
            val dx = event.rawX - downRawX
            val dy = point.y - start.y
            if (abs(dx) < slop && abs(dy) < slop) return
            mode = when {
                abs(dx) > abs(dy) && state.pages.size > 1 -> Mode.FLIP
                state.edge == PanelEdge.TOP && atTop() && dy > 0f -> Mode.PULL
                else -> Mode.NONE
            }
            if (mode == Mode.NONE) return
            if (mode == Mode.FLIP) flipBase = position()
            last = point
        }
        tracker.addPosition(event.eventTime, point)
        when (mode) {
            Mode.FLIP -> flipTo(flipBase - (event.rawX - downRawX) / width().coerceAtLeast(1f))
            Mode.PULL -> pulled += state.pull(point.y - last.y)
            Mode.NONE -> Unit
        }
        last = point
    }

    /** 移动事件被系统合并时，按这最后一下的屏幕坐标把少算的位移补齐。 */
    private fun catchUp(event: MotionEvent) {
        if (mode == Mode.FLIP) {
            flipTo(flipBase - (event.rawX - downRawX) / width().coerceAtLeast(1f))
        } else if (mode == Mode.PULL) {
            val raw = if (state.edge == PanelEdge.TOP) event.rawY - downRawY else downRawY - event.rawY
            pulled += state.pull(raw - pulled)
        }
    }

    private fun finish() {
        mode = Mode.NONE
        origin = null
    }
}
