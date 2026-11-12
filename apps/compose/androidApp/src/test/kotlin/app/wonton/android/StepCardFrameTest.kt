package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.FontStyle
import app.wonton.protocol.Rgb
import app.wonton.protocol.StepNav
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * StepNav.position 和 total 在 Kotlin 里是 UniFFI 给的 UInt，%1$d 不认它；把这两个字段直接交给
 * stringResource 会在组合时抛 IllegalFormatConversionException，而不是显示错的数字（#624）。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class StepCardFrameTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun stepProgressRenders() {
        compose.setContent {
            WontonTheme(testAppearance()) {
                StepCardFrame(
                    nav = StepNav(
                        position = 2u, total = 3u,
                        canBack = true, canSkip = false, canNext = true, canClose = false,
                    ),
                    title = "标题",
                    primaryText = "继续",
                    primaryBusy = false,
                    onBack = {},
                    onSkip = {},
                    onPrimary = {},
                ) {}
            }
        }

        compose.onNodeWithContentDescription("Step 2 of 3").assertIsDisplayed()
        compose.onNodeWithText("标题").assertIsDisplayed()
        compose.onNodeWithText("继续").assertIsDisplayed()
    }
}

internal fun rgb(hex: Int) = Rgb(
    (hex shr 16 and 0xFF).toUByte(),
    (hex shr 8 and 0xFF).toUByte(),
    (hex and 0xFF).toUByte(),
)

internal fun palette() = ThemeColors(
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
    accentFill = rgb(0xE3F2FD),
    accentInk = rgb(0x1565C0),
    accentInkOnFill = rgb(0x0D47A1),
    textOnAccentFill = rgb(0x0D47A1),
    accentFillBorder = rgb(0xBBDEFB),
)

internal fun testAppearance() = AppearanceViewModel(
    accent = rgb(0x3366FF),
    presets = listOf(rgb(0x3366FF)),
    moreColors = emptyList(),
    swatchColumns = 6u,
    themeMode = ThemeMode.SYSTEM,
    themeModes = listOf(ThemeMode.SYSTEM),
    textSize = TextSize.STANDARD,
    textSizes = listOf(TextSize.STANDARD),
    font = FontStyle.DEFAULT,
    fonts = listOf(FontStyle.DEFAULT),
    light = palette(),
    dark = palette(),
    problem = null,
)
