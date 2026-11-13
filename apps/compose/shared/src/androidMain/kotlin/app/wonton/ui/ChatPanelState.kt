package app.wonton.ui

import android.view.MotionEvent
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.input.pointer.motionEventSpy
import androidx.compose.runtime.DisposableEffect
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.Velocity
import app.wonton.ui.design.WontonTheme
import app.wonton.protocol.PanelItem
import app.wonton.protocol.PanelScope
import app.wonton.protocol.TabRef
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

internal enum class PanelLevel { CLOSED, PEEK, FULL }

/** 面板从哪一边出来：主聊天页在上（列表到顶后往下拉），房间在下（最新一条之后往上推）。 */
internal enum class PanelEdge { TOP, BOTTOM }

/** 第 1 级主聊天页保留 18%；房间保留约 45%，即面板占约 55%。 */
internal const val PANEL_MAIN_PEEK_FRACTION = 0.18f
internal const val PANEL_ROOM_PEEK_FRACTION = 0.45f

private const val SPRING_DAMPING = 0.86f
private const val SPRING_STIFFNESS = 340f

/**
 * 面板的级别、手指拖动和页面数据（ADR-0052）。拉到第几级、手指在哪是壳里的状态，不进核心。
 * 拖动方向以“把面板拉出来”为正：主聊天页往下，房间往上。
 */
@Suppress("TooManyFunctions") // 级别、手势、页面数据的同一处状态，拆开要到处传。
internal class ChatPanelState(
    val edge: PanelEdge,
    val scope: PanelScope,
    private val haptics: HapticFeedback,
    private val coroutine: CoroutineScope,
) {
    /** 聊天页相对自己位置的位移，0 关着，第 1 级是露出的那条之外的高度，第 2 级是整屏。 */
    private var shift by mutableFloatStateOf(0f)
    private var motion: Job? = null
    private var dragDistance = 0f
    var level by mutableStateOf(PanelLevel.CLOSED)
        private set
    var visible by mutableStateOf(false)
        private set
    var pages by mutableStateOf<List<PanelItem>>(emptyList())
        private set
    var selection by mutableStateOf<TabRef?>(null)
        private set
    var editing by mutableStateOf(false)
    var blocked by mutableStateOf(false)
    /** 聊天页往哪边移：主聊天页往下（正），房间往上（负）。 */
    val sign: Float get() = if (edge == PanelEdge.TOP) 1f else -1f

    var height = 0f
        private set

    /** 状态栏高度：主聊天页的聊天页在安全区里排版，第 1 级的位置要扣掉。 */
    var safeTop = 0f
        private set
    var safeBottom = 0f
        private set
    var density = 1f
        private set
    val dragSlop: Float get() = WontonTheme.gesture.dragSlop * density

    /** 开始时收起键盘，由宿主提供（键盘控制在组合里）。 */
    var onBegin: (() -> Unit)? = null

    /** 页面里的“发到聊天”要去哪：房间里是房间草稿，房间页负责换掉。 */
    var draftSink: ((String) -> Unit)? = null

    /** 当前这一页的网页控制器，系统返回先退网页历史。 */
    var controller: PluginWebViewController? = null
    private var armed = PanelLevel.CLOSED

    val shiftPx: Float get() = shift

    /** 第 1 级露出的那条的高度。 */
    val strip: Float get() = height * peekFraction()

    fun geometry(density: Float) {
        this.density = density
    }

    /** 布局后把尺寸带进来，拖动时按它算位置。 */
    fun ready(heightPx: Float, top: Float, bottom: Float) {
        height = heightPx
        safeTop = top
        safeBottom = bottom
    }

    /** 各级聊天页的位移。 */
    fun base(level: PanelLevel): Float = when (level) {
        PanelLevel.CLOSED -> 0f
        PanelLevel.PEEK -> height * (1 - peekFraction()) - if (edge == PanelEdge.TOP) safeTop else 0f
        PanelLevel.FULL -> height
    }
    private fun peekFraction(): Float =
        if (scope is PanelScope.Room) PANEL_ROOM_PEEK_FRACTION else PANEL_MAIN_PEEK_FRACTION

    fun clampedShift(): Float = shift.coerceIn(0f, height)

    /**
     * 核心推来新的页面；不认识的选中项落到第一页。房间的 blocked 由房间页报告。
     * 只在没拉开或换了一边时才会被外部改选中：拉开时用户自己翻的页优先。
     */
    fun feed(items: List<PanelItem>, saved: TabRef?) {
        pages = items
        if (level == PanelLevel.CLOSED && saved != null && items.any { it.tab == saved }) {
            selection = saved
        }
        if (selection != null && items.none { it.tab == selection }) selection = items.firstOrNull()?.tab
    }

    /** 拔出第一页：没有选中或已经不在时取第一页。 */
    fun begin() {
        if (visible) return
        visible = true
        if (selection == null || pages.none { it.tab == selection }) selection = pages.firstOrNull()?.tab
        onBegin?.invoke()
    }

    /** 手指拖动：往打开的方向为正，返回吃掉的距离。 */
    fun pull(delta: Float): Float {
        begin()
        motion?.cancel()
        val before = shift
        shift = (before + delta).coerceIn(0f, height)
        dragDistance += shift - before
        arm(level, dragDistance)
        return shift - before
    }

    /** Release uses actual travel; speed only qualifies a deliberate minimum-distance flick. */
    fun settle(from: PanelLevel, velocity: Float) {
        if (!visible) return
        move(target(from, dragDistance, velocity))
    }

    fun cancelDrag() {
        if (visible) move(level)
    }

    fun settleEdge(distance: Float, velocity: Float) {
        if (commitsGesture(distance, velocity, WontonTheme.gesture.swipeDistance, density)) {
            move(if (level == PanelLevel.FULL) PanelLevel.PEEK else PanelLevel.CLOSED)
        } else {
            cancelDrag()
        }
    }

    /** 直接换级（点露出的那条、胶囊退出、读屏动作）。 */
    fun move(to: PanelLevel) {
        dragDistance = 0f
        if (!visible) begin()
        val next = if (to == PanelLevel.FULL && pages.isEmpty()) PanelLevel.PEEK else to
        if (next != PanelLevel.CLOSED) visible = true
        level = next
        armed = next
        motion?.cancel()
        motion = coroutine.launch {
            animate(
                shift, base(next),
                animationSpec = spring(dampingRatio = SPRING_DAMPING, stiffness = SPRING_STIFFNESS),
            ) { value, _ -> shift = value }
            if (level == PanelLevel.CLOSED && shift <= 0f) visible = false
        }
    }

    /** 换边或换房间时直接落位，不带着上一处的拖动。 */
    fun reset() {
        motion?.cancel()
        shift = 0f
        dragDistance = 0f
        level = PanelLevel.CLOSED
        armed = PanelLevel.CLOSED
        visible = false
        selection = null
    }

    fun select(tab: TabRef) {
        selection = tab
    }

    /** 返回键：整页时先退网页历史和视频全屏，到头退回第 1 级；第 1 级收回第 0 级。 */
    fun back() {
        when (level) {
            PanelLevel.FULL -> {
                val handled = controller?.closeCustomView() == true || controller?.goBack() == true
                if (!handled) move(PanelLevel.PEEK)
            }
            else -> move(PanelLevel.CLOSED)
        }
    }

    private fun target(from: PanelLevel, distance: Float, velocity: Float = 0f): PanelLevel {
        val opens = commitsGesture(distance, velocity, WontonTheme.gesture.pullDistance, density)
        val pushes = commitsGesture(distance, velocity, WontonTheme.gesture.pushDistance, density)
        val closes = commitsGesture(-distance, -velocity, WontonTheme.gesture.pullDistance, density)
        return when (from) {
            PanelLevel.CLOSED -> when {
                pages.isNotEmpty() && distance >= base(PanelLevel.PEEK) +
                    WontonTheme.gesture.pushDistance * density -> PanelLevel.FULL
                opens -> PanelLevel.PEEK
                else -> PanelLevel.CLOSED
            }
            PanelLevel.PEEK -> when {
                pushes -> PanelLevel.FULL
                closes -> PanelLevel.CLOSED
                else -> PanelLevel.PEEK
            }
            PanelLevel.FULL -> if (closes) PanelLevel.PEEK else PanelLevel.FULL
        }
    }

    /** 跨过“松手就换一级”的位置时轻震一下，往回退不震。只看手指，不算速度。 */
    private fun arm(from: PanelLevel, distance: Float) {
        val next = target(from, distance)
        val was = armed
        armed = next
        if (next == was || next == from) return
        // 从第 0 级抬起时每跨过一次都算；往下换级只在往更深处时震。
        if (depth(next) > depth(was) || was == from) {
            haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        }
    }

    private fun depth(level: PanelLevel): Int = when (level) {
        PanelLevel.CLOSED -> 0
        PanelLevel.PEEK -> 1
        PanelLevel.FULL -> 2
    }

}

/** 当前这页的面板；不在聊天页时为空。 */
internal val LocalChatPanel = compositionLocalOf<ChatPanelState?> { null }

@Composable
internal fun rememberChatPanelState(edge: PanelEdge, scope: PanelScope): ChatPanelState {
    val haptics = LocalHapticFeedback.current
    val coroutine = rememberCoroutineScope()
    return remember(edge, scope) { ChatPanelState(edge, scope, haptics, coroutine) }
}

/**
 * 聊天列表到头后接着拉，把余下的手指距离交给面板（同系统下拉刷新的做法）。只认手指拖动：
 * 惯性滚到头的分量不算，照样留给列表。
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun Modifier.chatPanelPull(
    state: ChatPanelState?,
    atBoundary: () -> Boolean,
    enabled: Boolean,
): Modifier {
    if (state == null) return this
    DisposableEffect(state, enabled) { onDispose { state.cancelDrag() } }
    return motionEventSpy { event ->
        if (event.actionMasked == MotionEvent.ACTION_CANCEL) state.cancelDrag()
    }.nestedScroll(remember(state, enabled) {
        connection(state, atBoundary, enabled)
    })
}

private fun connection(
    state: ChatPanelState,
    atBoundary: () -> Boolean,
    enabled: Boolean,
): NestedScrollConnection =
    object : NestedScrollConnection {
        override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset {
            // 往后推的距离先把面板收回去，列表不跟着动。
            val closing = -available.y * state.sign
            val open = enabled && source == NestedScrollSource.UserInput &&
                state.level == PanelLevel.CLOSED && state.shiftPx > 0f
            if (!open || closing <= 0f) return Offset.Zero
            val consumed = state.pull(-closing)
            return Offset(0f, -consumed * state.sign)
        }

        override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
            val opening = available.y * state.sign
            val atRest = enabled && source == NestedScrollSource.UserInput &&
                state.level == PanelLevel.CLOSED && atBoundary()
            if (!atRest || opening <= 0f) return Offset.Zero
            val consumed = state.pull(opening)
            return Offset(0f, consumed * state.sign)
        }

        override suspend fun onPreFling(available: Velocity): Velocity {
            if (!enabled || state.level != PanelLevel.CLOSED || state.shiftPx <= 0f) return Velocity.Zero
            // The list's fling can consume the release speed even while its boundary is held by the panel.
            state.settle(PanelLevel.CLOSED, available.y * state.sign)
            return available
        }
    }
