package app.wonton.android

import android.Manifest
import android.os.Build
import android.view.KeyEvent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/** 主入口导航只用 Debug 的假核心，不连家里的服务器；每个测试都验证 Android 系统返回键。 */
@RunWith(AndroidJUnit4::class)
class MainNavigationTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    /** 假核心跟着进程活到整个类跑完：上一条停在应用网格（或中途失败）时，这一条仍从聊天根页面开始。 */
    @Before
    fun startFromChatRoot() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        assertChatRoot()
    }

    @Test
    fun testAppsSystemBackReturnsToChat() {
        openApps()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testAppsSettingsButtonOpensSettings() {
        openApps()
        compose.onNodeWithContentDescription("Settings").performClick()
        compose.onNodeWithText("Appearance").assertIsDisplayed()
        pressBack()
        compose.onNodeWithText("Notes").assertIsDisplayed()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testAppsStoreEntryAndPluginDetail() {
        openApps()
        compose.onNodeWithText("Store").performClick()
        compose.onNodeWithContentDescription("Add source").assertIsDisplayed()
        pressBack()
        compose.onNodeWithText("Notes").assertIsDisplayed()

        compose.onNodeWithText("Notes").performClick()
        compose.onNodeWithContentDescription("Settings").assertDoesNotExist()
        pressBack()
        compose.onNodeWithText("Notes").assertIsDisplayed()
    }

    @Test
    fun testStoreFromSidebar() {
        openFromSidebar("Store")
        compose.onNodeWithContentDescription("Add source").assertIsDisplayed()
        compose.onNodeWithContentDescription("Add source").performClick()
        compose.onNodeWithText("Install from file").assertIsDisplayed()
        pressBack()
        compose.onNodeWithText("Install from file").assertDoesNotExist()
        pressBack()
        compose.onNodeWithText("Notes").assertIsDisplayed()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testSettingsFromSidebar() {
        // 侧边栏的设置是只有图标的圆钮，没有文字。
        openSidebar()
        compose.onNodeWithTag("sidebar-settings").performClick()
        compose.onNodeWithText("Appearance").assertIsDisplayed()
        compose.onNodeWithText("Appearance").performClick()
        compose.onNodeWithContentDescription("Back").assertIsDisplayed()
        pressBack()
        compose.onNodeWithText("Appearance").assertIsDisplayed()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testChatRoomSystemBack() {
        compose.onNodeWithText("Echo").performClick()
        compose.onNodeWithContentDescription("Back").assertIsDisplayed()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testNewChatFromSidebarAndChatList() {
        openFromSidebar("New chat")
        compose.onNodeWithText("Start a direct chat").assertIsDisplayed()
        pressBack()
        assertChatRoot()

        // 聊天列表的新聊天是右下角只有图标的悬浮按钮。
        compose.onNodeWithContentDescription("New chat").performClick()
        compose.onNodeWithText("Start a direct chat").assertIsDisplayed()
        pressBack()
        assertChatRoot()
    }

    @Test
    fun testSidebarChatsRowReturnsToChatList() {
        openFromSidebar("Chats")
        assertChatRoot()

        openApps()
        pressBack()
        assertChatRoot()
        openSidebar()
        compose.onNodeWithText("My apps").assertIsDisplayed()
        compose.onNodeWithContentDescription("Collapse sidebar").performClick()
        assertChatRoot()
    }

    private fun openApps() {
        openFromSidebar("All")
        compose.onNodeWithText("Notes").assertIsDisplayed()
    }

    private fun openFromSidebar(label: String) {
        openSidebar()
        compose.onNodeWithText(label).performClick()
    }

    private fun waitDesc(description: String) {
        compose.waitUntil(20_000) {
            runCatching { compose.onNodeWithContentDescription(description).assertExists() }.isSuccess
        }
    }

    private fun openSidebar() {
        waitDesc("Sidebar")
        compose.onNodeWithContentDescription("Sidebar").performClick()
        compose.onNodeWithText("My apps").assertIsDisplayed()
    }

    private fun assertChatRoot() {
        waitDesc("Sidebar")
        compose.onNodeWithContentDescription("Sidebar").assertIsDisplayed()
        compose.onNodeWithText("Echo").assertIsDisplayed()
    }

    private fun pressBack() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
    }
}
