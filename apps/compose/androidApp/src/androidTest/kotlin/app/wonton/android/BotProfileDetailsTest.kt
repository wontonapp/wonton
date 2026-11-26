@file:Suppress("MagicNumber")

package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/** 资料页先给人看的东西，技术标识收在默认收起的“详情”里。 */
@RunWith(AndroidJUnit4::class)
class BotProfileDetailsTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Before
    fun startFromChatRoot() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Echo").fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun technicalIdentityStartsCollapsed() {
        // 先点开 Bot 的房间，再点顶栏头像进资料页（房间顶栏没有房子按钮，入口在资料页里）。
        compose.onNodeWithText("Echo").performClick()
        screenshot("step-room")
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
        compose.onNodeWithTag("bot-profile-open").performClick()
        screenshot("step-profile")
        compose.waitUntil(10_000) { compose.onNodeWithTag("bot-profile").isDisplayed() }
        screenshot("profile-top")
        // 插件 ID、账号、网页地址都要先点开才看得到。
        compose.onNodeWithText("app.example.echo", substring = true).assertDoesNotExist()
        compose.onNodeWithText("@echo:home", substring = true).assertDoesNotExist()
        compose.onNodeWithText("app-example-echo.wonton.localhost", substring = true).assertDoesNotExist()
        // 假核心给 Echo 开了网页：入口在详情外面，按钮就说清打开的是英文网页。
        compose.onNodeWithTag("bot-web-open").performScrollTo().assertIsDisplayed()
        screenshot("profile-action")
        compose.onNodeWithTag("bot-profile-details").performScrollTo().performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("app-example-echo.wonton.localhost", substring = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("app.example.echo", substring = true).assertIsDisplayed()
        compose.onNodeWithText("@echo:home", substring = true).assertIsDisplayed()
        compose.onNodeWithText("app-example-echo.wonton.localhost", substring = true).assertIsDisplayed()
        screenshot("profile-expanded")
    }

    private fun screenshot(name: String) {
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "bot-profile")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
