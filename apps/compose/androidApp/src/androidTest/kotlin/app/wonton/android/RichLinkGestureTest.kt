@file:Suppress("MagicNumber")

package app.wonton.android

import android.view.KeyEvent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.FontStyle
import app.wonton.protocol.RichBlock
import app.wonton.protocol.RichBlockKind
import app.wonton.protocol.RichLink
import app.wonton.protocol.RichLinkKind
import app.wonton.protocol.RichSpan
import app.wonton.protocol.Rgb
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.LinkOpener
import app.wonton.ui.LocalLinkOpener
import app.wonton.ui.R
import app.wonton.ui.RichText
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RichLinkGestureTest {
    @get:Rule
    val compose = createComposeRule()

    private val link = RichLink("https://example.com", RichLinkKind.External)
    private var openCount = 0
    private var opened: RichLink? = null
    private val opener = object : LinkOpener {
        override fun open(link: RichLink) {
            openCount += 1
            opened = link
        }

        override fun openInBrowser(link: RichLink) = error("Browser action was not selected")
        override fun share(link: RichLink) = error("Share action was not selected")
    }

    @Test
    fun verticalTouchMarginsKeepLongPressActionsAndOrdinaryTaps() {
        renderLink()
        val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density
        listOf(-4f * density, 4f * density).forEach { outside ->
            val bounds = compose.onNodeWithText(link.url).fetchSemanticsNode().boundsInRoot
            val y = if (outside < 0) bounds.top + outside else bounds.bottom + outside
            holdAt(Offset(bounds.center.x, y))
            assertActions()
            compose.runOnIdle { assertEquals("Long press must not open the URL", 0, openCount) }
            dismissActions()
        }
        repeat(4) {
            compose.onNodeWithText(link.url).performTouchInput { longClick() }
            assertActions()
            compose.runOnIdle { assertEquals(0, openCount) }
            dismissActions()
        }
        compose.onNodeWithText(link.url).performTouchInput { click() }
        compose.runOnIdle {
            assertEquals(1, openCount)
            assertEquals(link, opened)
        }
    }

    private fun renderLink() {
        val blocks = listOf(
            RichBlock(
                kind = RichBlockKind.Paragraph,
                depth = 0u,
                quoted = false,
                spans = listOf(RichSpan(link.url, false, false, false, false, link)),
            ),
        )
        val appearance = appearance()
        compose.setContent {
            WontonTheme(appearance) {
                CompositionLocalProvider(LocalLinkOpener provides opener) {
                    Box(Modifier.fillMaxSize().testTag("link-content").padding(64.dp)) {
                        RichText(blocks)
                    }
                }
            }
        }
        compose.onNodeWithText(link.url).assertIsDisplayed()
    }

    private fun holdAt(positionInRoot: Offset) {
        val container = compose.onNodeWithTag("link-content")
        val origin = container.fetchSemanticsNode().boundsInRoot.topLeft
        container.performTouchInput { longClick(positionInRoot - origin) }
    }

    private fun assertActions() {
        val label = InstrumentationRegistry.getInstrumentation().targetContext.getString(R.string.message_link_browser)
        compose.waitUntil(5_000) {
            runCatching { compose.onNodeWithText(label).assertIsDisplayed() }.isSuccess
        }
        compose.onNodeWithText(label).assertIsDisplayed()
    }

    private fun dismissActions() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        val label = InstrumentationRegistry.getInstrumentation().targetContext.getString(R.string.message_link_browser)
        compose.onNodeWithText(label).assertDoesNotExist()
    }

    private fun rgb(hex: Int) = Rgb(
        (hex shr 16 and 0xFF).toUByte(),
        (hex shr 8 and 0xFF).toUByte(),
        (hex and 0xFF).toUByte(),
    )

    private fun appearance(): AppearanceViewModel {
        val colors = ThemeColors(
            canvas = rgb(0xFFFFFF),
            surface = rgb(0xFFFFFF),
            surfaceSubtle = rgb(0xF2F2F2),
            textPrimary = rgb(0x000000),
            textSecondary = rgb(0x666666),
            separator = rgb(0xDDDDDD),
            primary = rgb(0x000000),
            onPrimary = rgb(0xFFFFFF),
            success = rgb(0x2E7D32),
            warning = rgb(0xF9A825),
            danger = rgb(0xC62828),
        )
        return AppearanceViewModel(
            themeMode = ThemeMode.SYSTEM,
            themeModes = listOf(ThemeMode.SYSTEM),
            textSize = TextSize.STANDARD,
            textSizes = listOf(TextSize.STANDARD),
            font = FontStyle.DEFAULT,
            fonts = listOf(FontStyle.DEFAULT),
            light = colors,
            dark = colors,
            problem = null,
        )
    }
}
