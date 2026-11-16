@file:Suppress("MagicNumber")

package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import android.view.KeyEvent
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith

/** 假核心给固定页面，本机 HTTP fixture 验证翻页；空面板另用真实开发栈走查。 */
@RunWith(AndroidJUnit4::class)
class RoomPanelGestureTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }
    private val failureEvidence = object : TestWatcher() {
        override fun failed(error: Throwable, description: Description) {
            screenshot("failure-${description.methodName}")
        }
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose).around(failureEvidence)

    private val openButton: SemanticsNodeInteraction
        get() = compose.onNodeWithContentDescription(compose.activity.getString(R.string.panel_open))
    private val closeStrip: SemanticsNodeInteraction
        get() = compose.onNode(
            hasContentDescription(compose.activity.getString(R.string.panel_close)) and hasClickAction(),
        )
    private val closePage: SemanticsNodeInteraction
        get() = compose.onNodeWithContentDescription(compose.activity.getString(R.string.tab_page_close))

    @Before
    fun startFromChatRoot() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        openButton.assertIsDisplayed()
    }

    @Test
    fun mainButtonOpensFullAndKeepsPagingAndEdgeClose() {
        canvasSwipe(0.5f, 0.7f, 0.5f, 0.25f)
        canvasSwipe(0.5f, 0.25f, 0.5f, 0.7f)
        openButton.assertIsDisplayed()
        closePage.assertDoesNotExist()
        closeStrip.assertIsNotDisplayed()
        openButton.performTouchInput { click() }
        closePage.assertIsDisplayed()
        closeStrip.assertIsNotDisplayed()
        compose.onNodeWithContentDescription("Player").performClick()
        // 假网页是普通 URL 页，没有 Bot 网页的登录披露；真 Bot 页有就必须先确认。
        val disclosure = compose.onAllNodesWithText(compose.activity.getString(R.string.bot_web_open))
        if (disclosure.fetchSemanticsNodes().isNotEmpty()) disclosure[0].performClick()
        compose.waitUntil(20_000) { visiblePlayer(instrumentation.uiAutomation.rootInActiveWindow) }
        screenshot("main-button-full")
        pressBack()
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        canvasSwipe(0.2f, 0.4f, 0.85f, 0.4f)
        compose.waitUntil(10_000) { !visiblePlayer(instrumentation.uiAutomation.rootInActiveWindow) }
        closeStrip.assertIsDisplayed()
        screenshot("main-peek-flipped")
        canvasSwipe(0.85f, 0.4f, 0.2f, 0.4f)
        compose.waitUntil(10_000) { visiblePlayer(instrumentation.uiAutomation.rootInActiveWindow) }
        closeStrip.assertIsDisplayed()
        canvasSwipe(0.005f, 0.5f, 0.85f, 0.5f)
        closeStrip.assertIsNotDisplayed()
        closePage.assertDoesNotExist()
        openButton.assertIsDisplayed()
        screenshot("main-edge-closed")
    }

    @Test
    fun roomSwipesStayClosedAndButtonBackAndStripWork() {
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
        val closedBottom = compose.onNode(hasSetTextAction()).fetchSemanticsNode().boundsInWindow.bottom
        val density = compose.activity.resources.displayMetrics.density
        pull(20f * density, 20)
        assertRoomClosed(closedBottom)
        pull(64f * density, 800)
        assertRoomClosed(closedBottom)
        pull(120f * density, 800)
        assertRoomClosed(closedBottom)
        pull(64f * density, 40)
        assertRoomClosed(closedBottom)
        openButton.performTouchInput { click() }
        closePage.assertIsDisplayed()
        screenshot("room-button-full")
        pressBack()
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        screenshot("room-back-peek")
        pressBack()
        assertRoomClosed(closedBottom)
        compose.onNodeWithContentDescription("Back").assertIsDisplayed()
        openButton.performTouchInput { click() }
        closePage.assertIsDisplayed()
        pressBack()
        closeStrip.performTouchInput { click() }
        assertRoomClosed(closedBottom)
        screenshot("room-strip-closed")
        pressBack()
        openButton.assertIsDisplayed()
        compose.onNode(hasSetTextAction()).assertIsNotDisplayed()
    }

    private fun pressBack() {
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
    }

    private fun canvasSwipe(x1: Float, y1: Float, x2: Float, y2: Float) {
        compose.onRoot().performTouchInput {
            swipe(Offset(width * x1, height * y1), Offset(width * x2, height * y2), durationMillis = 500)
        }
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

    private fun assertRoomClosed(originalBottom: Float) {
        val composer = compose.onNode(hasSetTextAction())
        composer.assertIsDisplayed()
        assertTrue("Closed panel must leave the composer in place",
            kotlin.math.abs(composer.fetchSemanticsNode().boundsInWindow.bottom - originalBottom) < 3f)
        closePage.assertDoesNotExist()
        closeStrip.assertIsNotDisplayed()
        openButton.assertIsDisplayed()
    }

    private fun visiblePlayer(node: AccessibilityNodeInfo?): Boolean {
        if (node == null) return false
        var found = node.isVisibleToUser && node.text?.startsWith("Fixture player") == true
        var index = 0
        while (!found && index < node.childCount) {
            found = visiblePlayer(node.getChild(index))
            index++
        }
        return found
    }

    private fun screenshot(name: String) {
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "panel-entry")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
