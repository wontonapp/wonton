package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 冒烟：主入口进得去、没崩。只断言“到了”，不断言位置、文案、颜色和手势细节；界面长什么样用截图验收
 * （AGENTS.md“测试”）。只用 Debug 的假核心，不连家里的服务器；按名字找元素只是为了点到它。
 */
@RunWith(AndroidJUnit4::class)
class SmokeTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    /** 假核心跟着进程活到整个类跑完：上一条停在别的页时，这一条仍从聊天列表开始。 */
    @Before
    fun startFromChatList() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        waitFor { compose.onNodeWithContentDescription("Sidebar").assertExists() }
    }

    @Test
    fun launchReachesChatList() {
        compose.onNodeWithText("Echo").assertExists()
    }

    @Test
    fun sidebarOpens() {
        openSidebar()
    }

    @Test
    fun chatSendsAMessage() {
        compose.onNodeWithText("Echo").performClick()
        waitFor { compose.onNode(hasSetTextAction()).assertExists() }
        compose.onNode(hasSetTextAction()).performTextInput("smoke")
        compose.onNodeWithContentDescription(compose.activity.getString(R.string.composer_send)).performClick()
        // 假核心把发出去的一条接在时间线末尾。
        waitFor { compose.onNode(hasTestTag("item-ui-fake-sent-1")).assertExists() }
    }

    @Test
    fun opensAnApp() {
        openSidebar()
        compose.onNodeWithText("All").performClick()
        waitFor { compose.onNodeWithText("Notes").assertExists() }
        compose.onNodeWithText("Notes").performClick()
        // 进了详情页：应用页顶上的设置不在了。
        waitFor { compose.onNodeWithContentDescription("Settings").assertDoesNotExist() }
    }

    @Test
    fun opensSettings() {
        openSidebar()
        compose.onNodeWithTag("sidebar-settings").performClick()
        waitFor { compose.onNodeWithText("Appearance").assertExists() }
    }

    private fun openSidebar() {
        compose.onNodeWithContentDescription("Sidebar").performClick()
        waitFor { compose.onNodeWithText("My apps").assertExists() }
    }

    private fun waitFor(check: () -> Unit) {
        compose.waitUntil(20_000) { runCatching(check).isSuccess }
    }
}
