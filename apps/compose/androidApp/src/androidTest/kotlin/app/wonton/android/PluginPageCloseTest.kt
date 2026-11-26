package app.wonton.android

import android.Manifest
import android.os.Build
import android.view.KeyEvent
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.ui.Problem
import app.wonton.ui.R
import app.wonton.ui.send
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 关闭整页应用（#696）：点关闭回到聊天列表，收尾的几下（连点、紧跟着的返回）不能把 App 摔到桌面。
 *
 * 「Corner」是假核心里带真网页的一页（`TabRef::Url`），整页上只有它还有胶囊；应用整页本身没有壳的控件（ADR-0095）。
 * 打开从「全部应用」进，和侧边栏“我的应用”是同一个打开路径。
 */
@RunWith(AndroidJUnit4::class)
class PluginPageCloseTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)
    /** 退出以后 `compose.activity` 会抛（场景已销毁），断言用同一个实例。 */
    private val activity by lazy { compose.activity }

    @Test
    fun testCloseReturnsToChatListAndStaysInTheApp() {
        openCorner()
        closePage().performClick()
        assertChatRoot()
        assertFalse(activity.isFinishing)
    }

    /** 关掉以后隔一下再按返回（手指的习惯动作）：返回落在根页面上，只提示“再按一次退出”，不退出。 */
    @Test
    fun testBackAfterTheCloseSettledStaysInTheApp() {
        openCorner()
        closePage().performClick()
        compose.waitForIdle()
        pressBack()
        assertFalse("关闭后隔一下再按返回不该退出 App", activity.isFinishing)
        assertChatRoot()
    }

    /** 关闭和返回几乎同时：页面的返回处理器还没撤掉，这一次由它接住，同样不退出。 */
    @Test
    fun testBackWhileThePageIsStillLeavingStaysInTheApp() {
        openCorner()
        closePage().performClick()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        assertFalse("关闭的同时按返回不该退出 App", activity.isFinishing)
        assertChatRoot()
    }

    /** 手指连点两下关闭：第二下落在空栈上，安静的什么也不做，不弹“没有打开的页面”。 */
    @Test
    fun testSecondCloseIsQuiet() {
        openCorner()
        val core = (activity.application as WontonApplication).core.getOrThrow()
        closePage().performClick()
        assertFalse(activity.isFinishing)

        var problem: Problem? = null
        compose.runOnUiThread { problem = core.send(Action.Pages(PageAction.ClosePage)) }
        assertNull("空栈上再关一次不该报错", problem)
        assertChatRoot()
    }

    /** 根页面第一下返回只提示，紧接着的第二下才退出。 */
    @Test
    fun testBackAtTheRootLeavesOnlyOnTheSecondPress() {
        assertChatRoot()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        assertFalse("第一下返回还在云吞里", activity.isFinishing)
        // 两下之间不空等：超过两秒的“再按一次”本来就作废。
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        assertTrue("第二下返回退出，而不是崩溃或卡住", activity.isFinishing)
    }

    private fun openCorner() {
        openSidebar()
        compose.onNodeWithText("All").performClick()
        waitForText("Corner")
        compose.onNodeWithText("Corner").performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithContentDescription(closeLabel).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private val closeLabel get() = activity.getString(R.string.tab_page_close)

    private fun closePage() = compose.onNodeWithContentDescription(closeLabel)

    private fun openSidebar() {
        waitForDescription("Sidebar")
        compose.onNodeWithContentDescription("Sidebar").performClick()
        waitForText("My apps")
    }

    private fun assertChatRoot() {
        waitForText("Echo")
        compose.onNodeWithContentDescription("Sidebar").assertExists()
    }

    private fun pressBack() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
    }

    private fun waitForText(text: String) {
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun waitForDescription(description: String) {
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithContentDescription(description).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private companion object {
        const val WAIT_MS = 10_000L
    }
}
