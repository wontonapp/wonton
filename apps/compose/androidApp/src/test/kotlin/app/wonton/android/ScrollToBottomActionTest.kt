package app.wonton.android

import android.app.Application
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.ui.design.ScrollToBottomAction
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** 回到底部按钮：命中区不小于 48dp 的点按门槛，数字进无障碍名字、超过 99 写 99+（ADR-0071）。 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], application = Application::class)
class ScrollToBottomActionTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun targetClearsHitMinimumEvenInATinySlot() {
        var taps = 0
        // 1dp 的骨架不给钮任何空间：门槛得由按钮自己撑出来，改小了这里先红。
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.size(1.dp)) {
                    ScrollToBottomAction(count = 3, onClick = { taps++ })
                }
            }
        }

        val button = compose.onNodeWithContentDescription("Back to bottom, 3 new messages")
        val bounds = button.getUnclippedBoundsInRoot()
        val width = bounds.right - bounds.left
        val height = bounds.bottom - bounds.top
        assertTrue("宽 $width 低于 48dp", width >= 48.dp)
        assertTrue("高 $height 低于 48dp", height >= 48.dp)
        button.performClick()
        assertEquals(1, taps)
    }

    @Test
    fun zeroCountNamesOnlyTheArrow() {
        compose.setContent {
            WontonTheme(testAppearance()) { ScrollToBottomAction(count = 0, onClick = {}) }
        }

        compose.onNodeWithContentDescription("Back to bottom").assertIsDisplayed()
    }

    @Test
    fun hundredNamesEveryMessageAndShows99Plus() {
        compose.setContent {
            WontonTheme(testAppearance()) { ScrollToBottomAction(count = 100, onClick = {}) }
        }

        compose.onNodeWithContentDescription("Back to bottom, 100 new messages").assertIsDisplayed()
        compose.onNodeWithText("99+").assertIsDisplayed()
    }
}
