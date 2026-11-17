package app.wonton.android

import android.app.Activity
import android.app.Instrumentation
import android.graphics.Rect
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.webkit.WebView
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import app.wonton.ui.R
import org.junit.Assert.assertTrue

private const val PAGE_WAIT_MILLIS = 10_000L
private const val PROBE_BYTE_LIMIT = 256
private const val UTF8_CONTINUATION_MASK = 0xc0
private const val UTF8_CONTINUATION_BYTE = 0x80

internal class PanelPageAssertions(
    private val compose: ComposeContentTestRule,
    private val instrumentation: Instrumentation,
    private val context: Activity,
) {
    fun assertInitialContent() {
        val title = context.getString(R.string.problem_invalid_input_title)
        compose.waitUntil(PAGE_WAIT_MILLIS) { hasDisplayedText(title) }
        val text = compose.onNodeWithText(title).fetchSemanticsNode()
            .config[SemanticsProperties.Text].joinToString("\n") { it.text }
        assertTrue("Initial page has no visible status", text.isNotBlank())
    }

    fun awaitPlayer() {
        compose.waitUntil(PAGE_WAIT_MILLIS) { playerVisible() }
    }

    /**
     * 状态页盖住网页时，旧文档的网页框仍是 `VISIBLE`（盖的是另一层不透明 Box），无障碍树里的旧文本也还在，
     * 所以不能用“看不到 Player”判断没就绪。壳自己的信号是状态页文字，等目标文字出现。
     *
     * 不能拿“加载占位消失”当判据：骨架（[R.string.problem_loading]）和那句话（[R.string.problem_slow_loading]）
     * 只在没有问题的状态页里出现，而这句 `SlowHint` 用 `alpha(0f)` 先占着位，图上看不见、语义树里算显示。
     */
    fun awaitNotReady(title: String) {
        compose.waitUntil(PAGE_WAIT_MILLIS) { hasDisplayedText(title) }
    }

    /** 网页文字在 WebView 的无障碍树里，只有网页框可见时才算页面上看得到。 */
    fun playerVisible(): Boolean =
        findPlayer(instrumentation.uiAutomation.rootInActiveWindow) != null && shownWebView()

    fun withPlayerHeld(assertHeld: () -> Unit) {
        val node = checkNotNull(findPlayer(instrumentation.uiAutomation.rootInActiveWindow)) {
            "Fixture player is not visible for the hold"
        }
        val bounds = Rect()
        node.getBoundsInScreen(bounds)
        check(!bounds.isEmpty) { "Fixture player has no touch bounds" }
        val downTime = SystemClock.uptimeMillis()
        val x = bounds.exactCenterX()
        val y = bounds.exactCenterY()
        sendPointer(MotionEvent.ACTION_DOWN, downTime, x, y)
        try {
            SystemClock.sleep(ViewConfiguration.getLongPressTimeout().toLong())
            assertHeld()
        } finally {
            sendPointer(MotionEvent.ACTION_UP, downTime, x, y)
        }
    }

    fun snapshot(): String {
        val text = StringBuilder(PROBE_BYTE_LIMIT)
        fun visit(node: AccessibilityNodeInfo?) {
            if (node == null || text.length >= PROBE_BYTE_LIMIT) return
            if (node.isVisibleToUser) node.text?.let { value ->
                if (text.isNotEmpty()) text.append('\n')
                text.append(value, 0, minOf(value.length, PROBE_BYTE_LIMIT - text.length))
            }
            for (index in 0 until node.childCount) {
                if (text.length >= PROBE_BYTE_LIMIT) break
                visit(node.getChild(index))
            }
        }
        visit(instrumentation.uiAutomation.rootInActiveWindow)
        val bytes = text.toString().toByteArray(Charsets.UTF_8)
        var end = minOf(bytes.size, PROBE_BYTE_LIMIT)
        while (
            end > 0 && end < bytes.size &&
            (bytes[end].toInt() and UTF8_CONTINUATION_MASK) == UTF8_CONTINUATION_BYTE
        ) end--
        return String(bytes, 0, end, Charsets.UTF_8)
    }

    private fun shownWebView(): Boolean {
        fun visit(view: View): Boolean = when {
            view is WebView -> view.isShown
            view is ViewGroup -> (0 until view.childCount).any { visit(view.getChildAt(it)) }
            else -> false
        }
        return visit(context.window.decorView)
    }

    private fun findPlayer(node: AccessibilityNodeInfo?): AccessibilityNodeInfo? {
        if (node == null) return null
        var found = if (node.isVisibleToUser && node.text?.startsWith("Fixture player") == true) node else null
        var index = 0
        while (found == null && index < node.childCount) {
            found = findPlayer(node.getChild(index))
            index++
        }
        return found
    }

    private fun sendPointer(action: Int, downTime: Long, x: Float, y: Float) {
        val event = MotionEvent.obtain(downTime, SystemClock.uptimeMillis(), action, x, y, 0)
        event.source = InputDevice.SOURCE_TOUCHSCREEN
        try {
            instrumentation.sendPointerSync(event)
        } finally {
            event.recycle()
        }
    }

    private fun hasDisplayedText(text: String): Boolean {
        val matches = compose.onAllNodesWithText(text)
        return matches.fetchSemanticsNodes().indices.any { matches[it].isDisplayed() }
    }
}
