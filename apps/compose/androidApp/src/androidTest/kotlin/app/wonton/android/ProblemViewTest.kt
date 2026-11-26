@file:Suppress("MagicNumber")

package app.wonton.android

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import java.io.File
import java.io.FileOutputStream
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.FontStyle
import app.wonton.protocol.Rgb
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.ProblemView
import app.wonton.ui.problem
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private const val CORE_TEXT = "reply could not be built: {error}"

/**
 * “还没起来”“输入不对”的整页错误：正文只留壳里写的一句话，核心原文收进默认收起的详情（ADR-0109 §7）。
 * 组件单独渲染：这两类错要真核心才碰得上，假数据进不了这个页面。浅色、深色各拍一张。
 */
@RunWith(AndroidJUnit4::class)
class ProblemViewTest {
    @get:Rule
    val compose = createComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun notReadyKeepsTheTextInDetails() = checkProblem(ChatFailure.NOT_READY, "not-ready")

    @Test
    fun invalidInputKeepsTheTextInDetails() = checkProblem(ChatFailure.INVALID_INPUT, "invalid-input")

    private fun checkProblem(failure: ChatFailure, name: String) {
        var mode by mutableStateOf(ThemeMode.LIGHT)
        compose.setContent {
            WontonTheme(appearance(mode)) {
                Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
                    ProblemView(failure.problem(CORE_TEXT))
                }
            }
        }
        val details = instrumentation.targetContext.getString(R.string.common_details)
        compose.onNodeWithText(CORE_TEXT).assertIsNotDisplayed()
        compose.onNodeWithText(details).assertIsDisplayed()
        shot("$name-light")
        compose.onNodeWithText(details).performClick()
        compose.onNodeWithText(CORE_TEXT).assertIsDisplayed()
        shot("$name-open-light")
        mode = ThemeMode.DARK
        compose.waitForIdle()
        compose.onNodeWithText(CORE_TEXT).assertIsDisplayed()
        shot("$name-open-dark")
    }

    private fun appearance(mode: ThemeMode) = AppearanceViewModel(
        accent = rgb(0x3366FF),
        presets = listOf(rgb(0x3366FF)),
        moreColors = emptyList(),
        swatchColumns = 6u,
        themeMode = mode,
        themeModes = listOf(mode),
        textSize = TextSize.STANDARD,
        textSizes = listOf(TextSize.STANDARD),
        font = FontStyle.DEFAULT,
        fonts = listOf(FontStyle.DEFAULT),
        light = palette(dark = false),
        dark = palette(dark = true),
        problem = null,
    )

    private fun palette(dark: Boolean): ThemeColors {
        val darkPalette = ThemeColors(
            canvas = rgb(0x121212),
            surface = rgb(0x1E1E1E),
            surfaceSubtle = rgb(0x2A2A2A),
            textPrimary = rgb(0xEDEDED),
            textSecondary = rgb(0xA0A0A0),
            separator = rgb(0x3A3A3A),
            primary = rgb(0xEDEDED),
            onPrimary = rgb(0x121212),
            success = rgb(0x81C784),
            warning = rgb(0xFFD54F),
            danger = rgb(0xEF5350),
            accentFill = rgb(0x1E3A5F),
            accentInk = rgb(0x90CAF9),
            accentInkOnFill = rgb(0xE3F2FD),
            textOnAccentFill = rgb(0xE3F2FD),
            accentFillBorder = rgb(0x2C4F7A),
        )
        val lightPalette = ThemeColors(
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
        return if (dark) darkPalette else lightPalette
    }

    private fun rgb(hex: Int) = Rgb(
        (hex shr 16 and 0xFF).toUByte(),
        (hex shr 8 and 0xFF).toUByte(),
        (hex and 0xFF).toUByte(),
    )

    private fun shot(name: String) {
        compose.waitForIdle()
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "problems")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
