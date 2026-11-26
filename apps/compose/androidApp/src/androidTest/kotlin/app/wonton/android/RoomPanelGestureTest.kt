@file:Suppress("MagicNumber")

package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.KeyEvent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.TouchInjectionScope
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.printToString
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.test.swipeWithVelocity
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
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

/** 快甩的注入时长（毫秒）：`swipeWithVelocity` 的下限是 40，再短就生成不出足够的事件。 */
private const val FLICK_MILLIS = 40L

/** 假核心给固定页面，本机 HTTP fixture 验证翻页；空面板另用真实开发栈走查。 */
@RunWith(AndroidJUnit4::class)
class RoomPanelGestureTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val pageAssertions by lazy {
        PanelPageAssertions(compose, instrumentation, compose.activity)
    }
    private val fixture by lazy { PanelPageFixture(compose.activity) }
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
        fixture.reset()
        // 上一个类留下的页面还要一帧才收掉，等根页真的画出来再断言。
        compose.waitUntil(10_000) { openButton.isDisplayed() }
        openButton.assertIsDisplayed()
    }

    /** 房间顶栏没有房子按钮，入口在 Bot 资料页。 */
    @Test
    fun panelOpensFromTheBotProfile() {
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
        openButton.assertDoesNotExist()
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        compose.onNodeWithTag("bot-profile").assertIsDisplayed()
        openButton.assertIsDisplayed()
        screenshot("bot-profile")
        openButton.performClick()
        awaitFull()
        closeStrip.assertIsNotDisplayed()
        screenshot("panel-from-profile")
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
        pageAssertions.assertInitialContent()
        compose.onNodeWithContentDescription("Player").performClick()
        // 假网页是普通 URL 页，没有 Bot 网页的登录披露；真 Bot 页有就必须先确认。
        val disclosure = compose.onAllNodesWithText(compose.activity.getString(R.string.bot_web_open))
        if (disclosure.fetchSemanticsNodes().isNotEmpty()) disclosure[0].performClick()
        awaitFullPlayer()
        screenshot("main-button-full")
        pressBack()
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        canvasSwipe(0.2f, 0.4f, 0.85f, 0.4f)
        compose.waitUntil(10_000) { !pageAssertions.playerVisible() }
        closeStrip.assertIsDisplayed()
        screenshot("main-peek-flipped")
        canvasSwipe(0.85f, 0.4f, 0.2f, 0.4f)
        compose.waitUntil(10_000) { pageAssertions.playerVisible() }
        closeStrip.assertIsDisplayed()
        canvasSwipe(0.005f, 0.5f, 0.85f, 0.5f)
        closeStrip.assertIsNotDisplayed()
        closePage.assertDoesNotExist()
        openButton.assertIsDisplayed()
        screenshot("main-edge-closed")
        openButton.performTouchInput { click() }
        // 重开这一刻渲染进程常正在崩（WebView 重建），点下去的落脚比平时慢，等面板真的回来再截图。
        compose.waitUntil(10_000) { closePage.isDisplayed() }
        // 收回后选中项会回落，重开的新渲染进程也常先以空白 finished：等回来的真页面，不拿一次 10 秒赌。
        awaitPlayerEventually()
        screenshot("main-reopened")
        pageAssertions.withPlayerHeld {
            awaitFullPlayer()
            screenshot("main-full-player-held")
        }
    }

    @Test
    fun uninstallingPageDoesNotReuseAStartedWebView() {
        openButton.performTouchInput { click() }
        compose.onNodeWithContentDescription("Player").performClick()
        awaitPlayerEventually()
        fixture.uninstallPlayer()
        SystemClock.sleep(15_000)
        val uninstalling = compose.activity.getString(R.string.tab_uninstalling)
        pageAssertions.awaitNotReady(uninstalling)
        compose.onNodeWithText(uninstalling).assertIsDisplayed()
        closePage.assertIsDisplayed()
        screenshot("started-uninstalling")
        fixture.removePlayer()
        val removed = compose.activity.getString(R.string.tab_page_removed)
        pageAssertions.awaitNotReady(removed)
        compose.onNodeWithText(removed).assertIsDisplayed()
        compose.onNodeWithText(compose.activity.getString(R.string.panel_remove)).assertIsDisplayed()
        pressBack()
        closeStrip.performTouchInput { click() }
        compose.waitUntil(10_000) { openButton.isDisplayed() }
        openButton.performTouchInput { click() }
        pageAssertions.awaitNotReady(removed)
        compose.onNodeWithText(removed).assertIsDisplayed()
        screenshot("removed-resumed")
        closePage.performTouchInput { click() }
    }

    @Test
    fun stoppedPageKeepsItsStatusAfterParkAndResume() {
        val tab = TabRef.Plugin("app.example.photos", null)
        val title = compose.activity.getString(R.string.tab_stopped, "Photos")
        // 「开始」按钮只在停机的插件页出现，和状态页标题一起断，别只认一个信号。
        val start = compose.activity.getString(R.string.tab_start)
        fixture.openPage(tab)
        pageAssertions.awaitNotReady(title)
        compose.onNodeWithText(start).assertIsDisplayed()
        screenshot("not-ready-open")
        fixture.openPage(TabRef.Chat)
        compose.waitUntil(10_000) { openButton.isDisplayed() }
        screenshot("not-ready-parked")
        fixture.openPage(tab)
        pageAssertions.awaitNotReady(title)
        compose.onNodeWithText(start).assertIsDisplayed()
        screenshot("not-ready-resumed")
    }

    /** 房间里的拖动不再拿起面板（ADR-0052 2026-11-16 起入口是按钮），各种距离和甩速都不许误开。 */
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
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        compose.onNodeWithTag("bot-profile").assertIsDisplayed()
        openButton.performTouchInput { click() }
        awaitFull()
        pageAssertions.awaitPlayer()
        screenshot("room-button-full")
        pressBack()
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        compose.waitUntil(10_000) { pageAssertions.playerVisible() }
        screenshot("room-back-peek")
        backToRoom()
        assertRoomClosed(closedBottom)
        compose.onNodeWithText("Echo").assertIsDisplayed()
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        openButton.performTouchInput { click() }
        awaitFull()
        pressBack()
        closeStrip.performTouchInput { click() }
        backToRoom()
        assertRoomClosed(closedBottom)
        screenshot("room-strip-closed")
        pressBack()
        openButton.assertIsDisplayed()
        compose.onNode(hasSetTextAction()).assertIsNotDisplayed()
    }

    /**
     * 面板换级照 ADR-0052 的门槛：走够 pushDistance / pullDistance，或者够 flickDistance 又够快；
     * 都不够就弹回原级、不算有意的拉。距离门槛用慢拖，速度门槛另用三倍 flickVelocity 的快甩单测。
     * 条带跟着面板走、行程本来就超过 pushDistance 和 pullDistance，由距离承担；顶部下拉区和左边缘不跟
     * 面板动，量到的末速才是注进去的。
     */
    @Test
    fun roomPanelChangesLevelOnlyPastDistanceOrFlick() {
        val density = compose.activity.resources.displayMetrics.density
        val push = WontonTheme.gesture.pushDistance * density
        val pull = WontonTheme.gesture.pullDistance * density
        // 注入的拖动先被系统的 touch slop 吃掉一截，门槛两边各留 24dp 的余量。
        fun below(threshold: Float) = threshold - 24f * density
        fun above(threshold: Float) = threshold + 32f * density

        openPanelFromProfile()
        awaitFullPlayer()
        // 整页时往下慢拖，差一截 pullDistance：弹回整页。
        dragPanelTop(below(pull), 600)
        closePage.assertIsDisplayed()
        // 快甩的行程够不到 pullDistance，只有够快才收起：回到第 1 级。
        dragPanelTop(flickTravel(density, pull), FLICK_MILLIS, flickVelocity())
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        // 第 1 级往上、往下各差一截门槛：都弹回第 1 级。
        dragStrip(-below(push), 600)
        closeStrip.assertIsDisplayed()
        dragStrip(below(pull), 600)
        closeStrip.assertIsDisplayed()
        // 慢拖走够 pushDistance 到整页：距离够就不看速度。
        dragStrip(-above(push), 600)
        awaitFull()
        // 整页时左边缘快甩：行程不到 swipeDistance，只有够快才合上，退到第 1 级。
        dragEdge(flickTravel(density, WontonTheme.gesture.swipeDistance * density), FLICK_MILLIS, flickVelocity())
        closeStrip.assertIsDisplayed()
        closePage.assertDoesNotExist()
        // 慢拖走够 pullDistance 合上。
        dragStrip(above(pull), 600)
        compose.waitUntil(10_000) { !closeStrip.isDisplayed() }
        // 面板是从 Bot 资料页打开的，合上后回到资料页。
        compose.onNodeWithTag("bot-profile").assertIsDisplayed()
        closePage.assertDoesNotExist()
    }

    private fun awaitFullPlayer() {
        pageAssertions.awaitPlayer()
        closePage.assertIsDisplayed()
        closeStrip.assertIsNotDisplayed()
        assertTrue("FULL must show the fixture page", pageAssertions.playerVisible())
    }

    /** WebView 渲染进程重建后旧页常停在空白；关掉再选一次，最多三轮。 */
    private fun awaitPlayerEventually() {
        repeat(3) {
            try {
                awaitFullPlayer()
                return
            } catch (_: ComposeTimeoutException) {
                Log.i("PanelGestureTest", "player blank tree:\n${compose.onRoot().printToString(maxDepth = 4)}")
                closePage.performTouchInput { click() }
                compose.waitUntil(10_000) { openButton.isDisplayed() }
                openButton.performTouchInput { click() }
                compose.waitUntil(10_000) { closePage.isDisplayed() }
                compose.onNodeWithContentDescription("Player").performClick()
            }
        }
        awaitFullPlayer()
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

    /** 点顶栏头像进 Bot 资料页，再从那里的按钮打开房间面板，等到整页。 */
    private fun openPanelFromProfile() {
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        compose.onNodeWithTag("bot-profile").assertIsDisplayed()
        openButton.performTouchInput { click() }
        awaitFull()
    }

    /** 在第 1 级露出的那条上匀速慢拖 `distance` 像素；正数朝下（收回）。 */
    private fun dragStrip(distance: Float, durationMillis: Long) {
        val density = compose.activity.resources.displayMetrics.density
        closeStrip.performTouchInput { swipeFrom(center, distance, durationMillis, null, density) }
        compose.waitForIdle()
    }

    /** 在整页面板顶上的那条下拉区里竖直拖，参数同 [dragStrip]。 */
    private fun dragPanelTop(distance: Float, durationMillis: Long, velocity: Float? = null) {
        val density = compose.activity.resources.displayMetrics.density
        compose.onRoot().performTouchInput {
            swipeFrom(
                Offset(width / 2f, WontonTheme.gesture.pageEdgeZone * density / 2f),
                distance, durationMillis, velocity, density,
            )
        }
        compose.waitForIdle()
    }

    /** 在左边缘竖线中点往右拖；边缘层不跟面板走、局部坐标就是屏幕坐标，快甩的末速不被抵消。 */
    private fun dragEdge(distance: Float, durationMillis: Long, velocity: Float? = null) {
        val density = compose.activity.resources.displayMetrics.density
        compose.onRoot().performTouchInput {
            swipeFrom(
                Offset(WontonTheme.gesture.edgeWidth * density / 2f, height * 0.58f),
                distance, durationMillis, velocity, density,
                horizontal = true,
            )
        }
        compose.waitForIdle()
    }

    /**
     * 甩的末速取令牌的三倍（dp/s）：面板跟着手指动会抵掉一截局部位移，量到的末速只剩一半上下，
     * 三倍才稳定过得了令牌门槛。产品门槛本身改大改小时，这里跟着令牌走。
     */
    private fun flickVelocity() = WontonTheme.gesture.flickVelocity * 3f

    /**
     * 快甩的行程（px）：`swipeWithVelocity` 从静止匀加速到末速，[FLICK_MILLIS] 内至少要末速一半的行程，
     * 否则报速度不可达，所以先按这个下限留 6dp 余量。行程还得留在 `ceiling`（距离门槛）下面，才证明是速度过的。
     * 门槛比这个下限还短时（产品门槛调高），行程会超出门槛，改由距离过。
     */
    private fun flickTravel(density: Float, ceiling: Float) =
        maxOf(
            ceiling - 4f * density,
            (flickVelocity() * FLICK_MILLIS / 2_000f + 6f) * density,
        )

    /** `velocity`（dp/s）为空时匀速慢拖 `durationMillis`；给了就按这个末速甩出去。 */
    private fun TouchInjectionScope.swipeFrom(
        start: Offset,
        distance: Float,
        durationMillis: Long,
        velocity: Float?,
        density: Float,
        horizontal: Boolean = false,
    ) {
        val end = start + (if (horizontal) Offset(distance, 0f) else Offset(0f, distance))
        if (velocity == null) {
            swipe(start, end, durationMillis)
        } else {
            swipeWithVelocity(start, end, velocity * density, durationMillis)
        }
    }

    /** 面板和资料页叠在房间上面，返回一下退一层；退到面板和资料页都收了为止。 */
    private fun backToRoom() {
        var left = 4
        while (left > 0 && layeredOverRoom()) {
            pressBack()
            compose.waitForIdle()
            left--
        }
    }

    private fun layeredOverRoom(): Boolean =
        closePage.isDisplayed() || closeStrip.isDisplayed() || compose.onNodeWithTag("bot-profile").isDisplayed()

    /** 面板展开是动画，关掉的页面按钮晚一点才有。 */
    private fun awaitFull() {
        compose.waitUntil(10_000) { closePage.isDisplayed() }
        closePage.assertIsDisplayed()
    }

    private fun assertRoomClosed(originalBottom: Float) {
        val composer = compose.onNode(hasSetTextAction())
        composer.assertIsDisplayed()
        compose.waitUntil(10_000) {
            kotlin.math.abs(composer.fetchSemanticsNode().boundsInWindow.bottom - originalBottom) < 3f
        }
        closePage.assertDoesNotExist()
        closeStrip.assertIsNotDisplayed()
        openButton.assertDoesNotExist()
    }

    private fun screenshot(name: String) {
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "panel-entry")
        check(directory.isDirectory || directory.mkdirs())
        File(directory, "$name.state.txt").writeText(pageAssertions.snapshot())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
