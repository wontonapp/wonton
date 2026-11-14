@file:Suppress("MagicNumber")

package app.wonton.android

import android.os.ParcelFileDescriptor
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.AgentRunState
import app.wonton.protocol.FontStyle
import app.wonton.protocol.Rgb
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.AgentRunRowView
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 夜跑界面测试（ADR-0082）：working 显示工作标记和步骤、一轮收成一行“N 步”，waiting 整行可点开卡片。
 * 时间线要登录后的房间才有（核心假数据只够首屏），所以直接渲染真组件，数据用核心类型构造。
 */
@RunWith(AndroidJUnit4::class)
class AgentRunRowTest {
    @get:Rule
    val compose = createComposeRule()

    /** 夜跑前读到的动画时长缩放原值，@After 按它还原。 */
    private var animatorScale: String? = null

    @Before
    fun useAnimations() {
        // 夜跑模拟器整体关动画（nightly-ui 的 disable-animations），组件按 ADR-0044 在减弱动态
        // 效果时不画转圈；本用例断言的正是转圈，所以打开，结束后按原值还原，别的用例不背这个改动。
        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation
        animatorScale = ParcelFileDescriptor.AutoCloseInputStream(
            ui.executeShellCommand("settings get global animator_duration_scale"),
        ).bufferedReader().use { it.readText().trim() }
        ui.executeShellCommand("settings put global animator_duration_scale 1")
    }

    @After
    fun restoreAnimations() {
        val scale = animatorScale ?: return
        val cmd = if (scale.isEmpty() || scale == "null") {
            "settings delete global animator_duration_scale"
        } else {
            "settings put global animator_duration_scale $scale"
        }
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(cmd)
    }

    @Test
    fun workingToDoneAndWaitingOpensCard() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        var openedCardId: String? = null
        // 工作状态可能是转圈或静态图标；停掉帧钟，阶段之间手动走一帧。
        // 三个阶段共用一次 setContent（ui-test 不允许设第二次），阶段之间改状态。
        compose.mainClock.autoAdvance = false
        var run by mutableStateOf(fixture(AgentRunState.Working, step = "Reading files"))
        compose.setContent {
            // AgentRunRowView 取 WontonTheme 的令牌，CompositionLocal 没有默认值，测试里自己提供一套。
            WontonTheme(previewAppearance()) {
                AgentRunRowView(run, { openedCardId = it }, {})
            }
        }
        // reduceMotion 的系统监听是异步进来的：先等它落进组合，再走帧断言。
        compose.waitForIdle()
        compose.mainClock.advanceTimeBy(100)
        compose.onNodeWithTag("agentRunWorkingMark").assertExists()
        compose.onNodeWithText("Reading files").assertExists()

        run = fixture(AgentRunState.Done, steps = 5u)
        compose.mainClock.advanceTimeBy(100)
        compose.onNodeWithTag("agentRunWorkingMark").assertDoesNotExist()
        compose.onNodeWithText(context.resources.getQuantityString(R.plurals.agent_run_steps, 5, 5))
            .assertExists()

        run = waiting()
        compose.mainClock.advanceTimeBy(100)
        compose.onNodeWithText(context.getString(R.string.agent_run_waiting)).performClick()
        compose.waitForIdle()
        assertEquals("card-1", openedCardId)
    }

    private fun fixture(
        state: AgentRunState,
        step: String? = null,
        steps: UInt? = null,
        cardId: String? = null,
    ) = TimelineContent.AgentRun(
        run = null,
        state = state,
        step = step,
        steps = steps,
        progress = null,
        result = null,
        end = null,
        body = "",
        cardId = cardId,
        choices = emptyList(),
        senderName = "Echo",
    )

    private fun waiting() = fixture(AgentRunState.Waiting, cardId = "card-1")

    // 主题夹具照 QualityPreviewActivity 的写法抄了一份：AgentRunRowView 没有令牌提供就抛错。
    private fun rgb(hex: Int) = Rgb(
        (hex shr 16 and 0xFF).toUByte(),
        (hex shr 8 and 0xFF).toUByte(),
        (hex and 0xFF).toUByte(),
    )

    private fun palette() = ThemeColors(
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

    private fun previewAppearance() = AppearanceViewModel(
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
}
