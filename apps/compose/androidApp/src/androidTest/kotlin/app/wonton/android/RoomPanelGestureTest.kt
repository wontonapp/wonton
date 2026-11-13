@file:Suppress("MagicNumber")

package app.wonton.android

import android.view.KeyEvent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.ui.R
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeNotNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Run against an already-paired disposable backend with -e roomName <joined writable room>. */
@RunWith(AndroidJUnit4::class)
class RoomPanelGestureTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    @Test
    fun longTimelineCommitsOnlyDeliberatePulls() {
        val room = InstrumentationRegistry.getArguments().getString("roomName")
        assumeNotNull(room)
        compose.waitUntil(60_000) {
            compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().isNotEmpty() ||
                runCatching { compose.onNodeWithText(requireNotNull(room)).fetchSemanticsNode() }.isSuccess
        }
        if (compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText(requireNotNull(room)).performClick()
        }
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
        val composer = compose.onNode(hasSetTextAction())
        val message = "Panel boundary ${System.nanoTime()}: " + "Long timeline verification. ".repeat(40)
        composer.performClick().performTextReplacement(message)
        compose.onNodeWithContentDescription(compose.activity.getString(R.string.composer_send)).performClick()
        pressBack()
        compose.waitUntil(30_000) {
            runCatching { compose.onNodeWithText(message, substring = true).fetchSemanticsNode() }.isSuccess
        }
        compose.waitForIdle()
        val scroll = compose.onNodeWithTag("room-timeline")
        val range = scroll.fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange]
        assertTrue("Room must overflow the viewport to catch reversed-boundary bugs", range.maxValue() > 0)
        val closedBottom = composer.fetchSemanticsNode().boundsInWindow.bottom
        val density = compose.activity.resources.displayMetrics.density

        pull(20f * density, 20)
        assertClosed(closedBottom)
        pull(64f * density, 800)
        assertClosed(closedBottom)
        pull(120f * density, 800)
        assertOpen(closedBottom)
        pressBack()
        assertClosed(closedBottom)
        pull(64f * density, 40)
        assertOpen(closedBottom)
        pressBack()
        assertClosed(closedBottom)
    }

    private fun pressBack() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
    }

    private fun pull(distance: Float, duration: Long) {
        val timeline = compose.onNodeWithTag("room-timeline")
        val bounds = timeline.fetchSemanticsNode().boundsInWindow
        val composerTop = compose.onNode(hasSetTextAction()).fetchSemanticsNode().boundsInWindow.top
        val unobscuredHeight = minOf(bounds.bottom, composerTop) - bounds.top
        timeline.performTouchInput {
            val start = Offset(bounds.width / 2f, unobscuredHeight / 2f)
            swipe(start, start - Offset(0f, distance), durationMillis = duration)
        }
        compose.waitForIdle()
    }

    private fun assertClosed(originalBottom: Float) {
        val bottom = compose.onNode(hasSetTextAction()).fetchSemanticsNode().boundsInWindow.bottom
        assertTrue("Short pull must restore the composer", kotlin.math.abs(bottom - originalBottom) < 3f)
    }

    private fun assertOpen(originalBottom: Float) {
        val bottom = compose.onNode(hasSetTextAction()).fetchSemanticsNode().boundsInWindow.bottom
        assertTrue("Deliberate pull at the newest message must reveal the panel", bottom < originalBottom * 0.6f)
    }
}
