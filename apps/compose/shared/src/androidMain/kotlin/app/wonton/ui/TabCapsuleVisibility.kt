package app.wonton.ui

import android.view.MotionEvent
import android.view.accessibility.AccessibilityManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.abs

private const val CAPSULE_IDLE_MS = 3_000L
private val capsuleDragThreshold = 24.dp

internal class TabCapsuleVisibility {
    var shown by mutableStateOf(true)
        private set
    var showings by mutableIntStateOf(0)
        private set
    var exploring by mutableStateOf(false)
    var held by mutableStateOf(false)
    var menuOpen by mutableStateOf(false)

    val visible: Boolean get() = shown || exploring
    val idle: Boolean get() = !exploring && !held

    fun show() { shown = true; showings++ }
    fun hide() { shown = false }
}

@Composable
internal fun TabCapsuleEffects(
    state: TabCapsuleVisibility, active: Boolean, fullscreen: Boolean,
) {
    val configuration = LocalConfiguration.current
    val width = configuration.screenWidthDp
    val height = configuration.screenHeightDp
    val context = LocalContext.current
    val manager = remember(context) { context.getSystemService(AccessibilityManager::class.java) }
    DisposableEffect(manager) {
        state.exploring = manager.isTouchExplorationEnabled
        val listener = AccessibilityManager.TouchExplorationStateChangeListener { state.exploring = it }
        manager.addTouchExplorationStateChangeListener(listener)
        onDispose { manager.removeTouchExplorationStateChangeListener(listener) }
    }
    LaunchedEffect(active, fullscreen, width, height, state.exploring) {
        if (active) state.show()
    }
    LaunchedEffect(active, fullscreen, state.showings, state.exploring, state.held) {
        if (active && state.idle) {
            delay(CAPSULE_IDLE_MS)
            state.hide()
        }
    }
}

@Composable
internal fun rememberCapsuleTouchObserver(
    state: TabCapsuleVisibility, enabled: Boolean,
): ((MotionEvent) -> Unit)? {
    val threshold = with(LocalDensity.current) { capsuleDragThreshold.toPx() }
    val gesture = remember(state, threshold) { CapsuleDragObserver(threshold, state) }
    return remember(gesture, enabled) { if (enabled) gesture::observe else null }
}

private class CapsuleDragObserver(private val threshold: Float, private val state: TabCapsuleVisibility) {
    private var lastY = 0f
    private var run = 0f
    private var sent: Boolean? = null
    private var tracking = false

    fun observe(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                tracking = true
                lastY = event.y
                run = 0f
                sent = null
            }
            MotionEvent.ACTION_MOVE -> if (tracking) move(event.y)
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN -> tracking = false
        }
    }

    private fun move(y: Float) {
        val step = y - lastY
        lastY = y
        if (step == 0f) return
        if ((step > 0) != (run > 0)) run = 0f
        run += step
        val down = run > 0
        if (abs(run) >= threshold && sent != down) {
            sent = down
            if (down) state.show() else state.hide()
        }
    }
}
