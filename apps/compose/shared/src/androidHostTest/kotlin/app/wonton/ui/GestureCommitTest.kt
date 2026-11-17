@file:Suppress("MagicNumber")

package app.wonton.ui

import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GestureCommitTest {
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

    /** 网页里面起手的下拉归网页（全屏浮层、播放器），只有露出的顶边附近才拖面板。 */
    @Test
    fun pagePullOnlyStartsNearTheVisibleTop() {
        for (density in listOf(1f, 2f, 3f)) {
            val zone = WontonTheme.gesture.pageEdgeZone * density
            val statusBar = 24f * density
            assertTrue(pagePullStartsInZone(0f, 0f, density))
            assertTrue(pagePullStartsInZone(zone, 0f, density))
            assertFalse(pagePullStartsInZone(zone + 1f, 0f, density))
            // 铺满的页面：状态栏下面那一截也算，起手区从露出的顶边往下量。
            assertTrue(pagePullStartsInZone(statusBar + zone, statusBar, density))
            assertFalse(pagePullStartsInZone(statusBar + zone + 1f, statusBar, density))
        }
    }
}
