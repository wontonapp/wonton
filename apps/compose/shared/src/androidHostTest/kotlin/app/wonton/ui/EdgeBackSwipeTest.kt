@file:Suppress("MagicNumber")

package app.wonton.ui

import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Test

/** 左边缘返回只在系统那条外面补：系统区吃得越多，这条越窄；吃满 24dp 就不留（#931）。 */
class EdgeBackSwipeTest {
    @Test
    fun systemAreaIsCutOutOfTheZone() {
        for (density in listOf(1f, 2f, 2.625f, 3f)) {
            val zone = WontonTheme.gesture.edgeWidth * density
            assertEquals(zone, edgeBackWidthPx(WontonTheme.gesture.edgeWidth, 0f, density), 0.01f)
            assertEquals(zone / 2f, edgeBackWidthPx(WontonTheme.gesture.edgeWidth, zone / 2f, density), 0.01f)
        }
    }

    /** 系统区比 24dp 还宽（把返回调更灵敏的机型）：不留负宽度。 */
    @Test
    fun systemAreaWiderThanTheZoneLeavesNothing() {
        val zoneAt3x = WontonTheme.gesture.edgeWidth * 3f
        assertEquals(0f, edgeBackWidthPx(WontonTheme.gesture.edgeWidth, zoneAt3x, 3f), 0f)
        assertEquals(0f, edgeBackWidthPx(WontonTheme.gesture.edgeWidth, zoneAt3x + 40f, 3f), 0f)
    }

    /** 方向要走过触摸阈值才定；吃不准以前谁也不动，平手算竖向、让给列表。 */
    @Test
    fun axisStaysUndecidedUntilTheSlopIsPassed() {
        assertEquals(DragAxis.Undecided, axisAfter(dx = 7f, dy = 7f, slop = 8f))
        assertEquals(DragAxis.Horizontal, axisAfter(dx = 9f, dy = 2f, slop = 8f))
        assertEquals(DragAxis.Vertical, axisAfter(dx = 2f, dy = 9f, slop = 8f))
        assertEquals(DragAxis.Vertical, axisAfter(dx = 9f, dy = 9f, slop = 8f))
    }

    /** 往回拖（负的横向位移）也算横向，只是最后不够长、不返回。 */
    @Test
    fun axisIgnoresTheDirection() {
        assertEquals(DragAxis.Horizontal, axisAfter(dx = -9f, dy = 1f, slop = 8f))
    }
}
