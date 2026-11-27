package app.wonton.android

import android.Manifest
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.MotionEvent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import org.junit.After
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * #931 的 Android 半：左边缘 24dp 内起手右滑 = 返回，同一条边上的拖动不再被列表行当成点按。
 *
 * 注入的是真 MotionEvent（经 `UiAutomation.injectInputEvent`，和手指走同一条输入路径），
 * 所以系统自己那条手势和 App 补的 [系统区, 24dp) 那一段都算在内。
 * 系统区在模拟器上约 30dp、比 24dp 还宽，把 App 要补的那一段盖掉了：先按返回灵敏度把它缩到 0.4。
 */
@RunWith(AndroidJUnit4::class)
class EdgeBackTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    private var savedScale: String? = null

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Before
    fun startFromChatRootWithANarrowSystemArea() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        savedScale = shell("settings get secure back_gesture_inset_scale_left").trim()
        shell("settings put secure back_gesture_inset_scale_left 0.4")
        // 系统按设置更新识别区要一两帧，App 那边是内容观察者跟的。
        SystemClock.sleep(1_000)
        compose.waitForIdle()
    }

    @After
    fun restoreSystemArea() {
        val scale = savedScale
        val command = if (scale.isNullOrEmpty() || scale == "null") {
            "settings delete secure back_gesture_inset_scale_left"
        } else {
            "settings put secure back_gesture_inset_scale_left $scale"
        }
        shell(command)
    }

    @Test
    fun swipeFromTheSystemAreaPopsTheStorePage() {
        openStore()
        swipeFrom(xInSystemArea(), rowY("Home Music"), px(100f))
        awaitGone("Photo Vault")
    }

    /** 起手落在系统区外面、24dp 里面：系统不接，只有 App 补的那一段能接。 */
    @Test
    fun swipeFromTheBandPopsTheStorePage() {
        openStore()
        swipeFrom(xInBand(), rowY("Home Music"), px(100f))
        awaitGone("Photo Vault")
    }

    /** 24dp 外面拖动不是返回：不前进也不后退，更不能把行点开。 */
    @Test
    fun dragOutsideTheBandOnARowDoesNotOpenIt() {
        openStore()
        swipeFrom(px(32f), rowY("Home Music"), px(100f))
        compose.waitForIdle()
        compose.onNodeWithText(DETAIL).assertDoesNotExist()
        compose.onNodeWithText("Photo Vault").assertIsDisplayed()
    }

    /** 同一段里点一下还是点按：行照常打开。 */
    @Test
    fun tapInTheBandOnARowStillOpensIt() {
        openStore()
        val x = xInBand()
        val y = rowY("Home Music")
        val downTime = SystemClock.uptimeMillis()
        inject(MotionEvent.obtain(downTime, downTime, MotionEvent.ACTION_DOWN, x, y, 0))
        inject(MotionEvent.obtain(downTime, downTime + 40L, MotionEvent.ACTION_UP, x, y, 0))
        awaitText(DETAIL)
    }

    /**
     * 聊天根本来有一条左边缘开侧边栏的条（[0, 24dp)、屏高中间那一段），App 补的这段不能把它抢掉。
     * 起点挑在系统区外面，测的是那条条本身：系统区里归系统。
     */
    @Test
    fun edgeSwipeOnTheChatRootStillOpensTheSidebar() {
        swipeFrom(px(19f), screenCenterY(), px(160f))
        awaitText("My apps")
    }

    // ---- 导航 ----

    private fun openStore() {
        awaitContentDescription("Sidebar")
        compose.onNodeWithContentDescription("Sidebar").performClick()
        awaitText("Store")
        compose.onNodeWithText("Store").performClick()
        awaitText("Photo Vault")
    }

    // ---- 手势 ----

    /** 从 (fromX, y) 横着拖 `distance` 像素；事件时间按 180ms 铺开，够慢、够远。 */
    private fun swipeFrom(fromX: Float, y: Float, distance: Float) {
        val steps = 12
        val downTime = SystemClock.uptimeMillis()
        inject(MotionEvent.obtain(downTime, downTime, MotionEvent.ACTION_DOWN, fromX, y, 0))
        for (step in 1..steps) {
            val x = fromX + distance * step / steps
            inject(MotionEvent.obtain(downTime, downTime + 15L * step, MotionEvent.ACTION_MOVE, x, y, 0))
            SystemClock.sleep(20)
        }
        val upTime = downTime + 15L * (steps + 1)
        inject(MotionEvent.obtain(downTime, upTime, MotionEvent.ACTION_UP, fromX + distance, y, 0))
    }

    private fun inject(event: MotionEvent) {
        instrumentation.uiAutomation.injectInputEvent(event, true)
        event.recycle()
    }

    private fun px(dp: Float): Float = dp * compose.activity.resources.displayMetrics.density

    /** 系统自己那条手势的右沿（px）。WM 报的宽度不跟灵敏度走，得自己乘。 */
    private fun systemAreaEnd(): Float {
        val view = compose.activity.window.decorView
        val insets = ViewCompat.getRootWindowInsets(view) ?: return 0f
        return insets.getInsets(WindowInsetsCompat.Type.systemGestures()).left * 0.4f
    }

    private fun xInSystemArea(): Float = (systemAreaEnd() / 2f).coerceAtLeast(px(1f))

    private fun xInBand(): Float {
        val x = systemAreaEnd() + px(2f)
        assumeTrue("系统区盖满了 24dp 起手区，这条用例不适用", x < px(24f))
        return x
    }

    /** 行文字的中心 y（窗口坐标）。 */
    private fun rowY(text: String): Float = compose.onNodeWithText(text).fetchSemanticsNode().boundsInWindow.center.y

    private fun screenCenterY(): Float = compose.onRoot().fetchSemanticsNode().boundsInWindow.center.y

    // ---- 等待 ----

    private fun awaitText(text: String) {
        compose.waitUntil(TIMEOUT) {
            runCatching { compose.onNodeWithText(text).fetchSemanticsNode() }.isSuccess
        }
    }

    private fun awaitGone(text: String) {
        compose.waitUntil(TIMEOUT) {
            runCatching { compose.onNodeWithText(text).assertDoesNotExist() }.isSuccess
        }
    }

    private fun awaitContentDescription(description: String) {
        compose.waitUntil(TIMEOUT) {
            runCatching { compose.onNodeWithContentDescription(description).assertExists() }.isSuccess
        }
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor.AutoCloseInputStream(
            instrumentation.uiAutomation.executeShellCommand(command),
        ).bufferedReader().use { it.readText() }

    private companion object {
        const val TIMEOUT = 10_000L
        const val DETAIL = "The household's records, playable from anywhere"
    }
}
