@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import app.wonton.protocol.PanelScope
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import org.junit.Assert.assertEquals
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GestureCommitTest {
    @Test
    fun cancelledLongPullKeepsInitialLevel() {
        for (edge in PanelEdge.entries) {
            val state = panel(edge)
            state.pull(WontonTheme.gesture.pullDistance * 2f)
            state.cancelDrag()
            assertEquals(PanelLevel.CLOSED, state.level)
        }
    }

    @Test
    fun homeAndRoomBothRequireInitialPullDistance() {
        for (edge in PanelEdge.entries) {
            val state = panel(edge)
            state.pull(WontonTheme.gesture.pullDistance - 1f)
            state.settle(PanelLevel.CLOSED, 0f)
            assertEquals(PanelLevel.CLOSED, state.level)
            val fullTravel = panel(edge)
            fullTravel.pull(WontonTheme.gesture.pullDistance)
            fullTravel.settle(PanelLevel.CLOSED, 0f)
            assertEquals(PanelLevel.PEEK, fullTravel.level)
        }
    }

    private fun panel(edge: PanelEdge): ChatPanelState {
        val haptics = object : HapticFeedback {
            override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) = Unit
        }
        // These tests observe the committed level, not animation frames.
        val scope = CoroutineScope(Job().apply { cancel() })
        return ChatPanelState(edge, PanelScope.Main, haptics, scope).apply {
            geometry(1f)
            ready(1000f, 0f, 0f)
        }
    }

    @Test
    fun tinyFastFlickNeverCommitsAtAnyDensity() {
        for (density in listOf(1f, 2f, 3f)) {
            assertFalse(commitsGesture(0f, 10000f * density, WontonTheme.gesture.pullDistance, density))
            assertFalse(
                commitsGesture(
                    WontonTheme.gesture.flickDistance * density - 1f,
                    WontonTheme.gesture.flickVelocity * density * 2f,
                    WontonTheme.gesture.pullDistance, density,
                ),
            )
        }
    }

    @Test
    fun slowTravelCommitsAtFullDistanceOnly() {
        for (threshold in listOf(
            WontonTheme.gesture.pullDistance, WontonTheme.gesture.pushDistance, WontonTheme.gesture.swipeDistance,
        )) {
            for (density in listOf(1f, 2f, 3f)) {
                assertFalse(commitsGesture(threshold * density - 1f, 0f, threshold, density))
                assertTrue(commitsGesture(threshold * density, 0f, threshold, density))
            }
        }
    }

    @Test
    fun deliberateFlickRequiresDirectedReleaseSpeed() {
        for (density in listOf(1f, 2f, 3f)) {
            val distance = WontonTheme.gesture.flickDistance * density
            val velocity = WontonTheme.gesture.flickVelocity * density
            val threshold = WontonTheme.gesture.pullDistance
            assertFalse(commitsGesture(distance, velocity - 1f, threshold, density))
            assertFalse(commitsGesture(distance, -velocity, threshold, density))
            assertFalse(commitsGesture(-distance, velocity, threshold, density))
            assertTrue(commitsGesture(distance, velocity, threshold, density))
        }
    }
}
