package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import app.wonton.ui.R
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 房间的通话入口（ADR-0096），只用 Debug 的假核心（`ui_fake.rs` 里有人在通话的群聊 Family）：聊天列表那一行标着通话中；
 * 进房间顶栏有入口，点了全屏打开 widget 页，页上有出口和中继提示，关掉回到房间；没装 widget 的房间没有入口。
 * 假核心没有隧道，widget 网页本身加载不出来；真通话在开发栈上走。
 */
@RunWith(AndroidJUnit4::class)
class CallEntryTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun testListRowMarksOnlyTheRoomInCall() {
        waitForTag("room-row-call-active", unmerged = true)
        compose.onAllNodesWithTag("room-row-call-active", useUnmergedTree = true).assertCountEquals(1)
    }

    @Test
    fun testEntryOpensWidgetPageAndExitReturnsToTheRoom() {
        compose.onNodeWithText("Family").performClick()
        waitForTag("room-call-active")
        compose.onNodeWithTag("room-call-active").performClick()

        val close = compose.activity.getString(R.string.tab_page_close)
        compose.waitUntil(WAIT_MS) {
            runCatching { compose.onNodeWithContentDescription(close).assertIsDisplayed() }.isSuccess
        }
        compose.onNodeWithTag("page-hint").assertIsDisplayed()

        compose.onNodeWithContentDescription(close).performClick()
        waitForTag("room-call-active")
        compose.onNodeWithTag("room-call-active").assertIsDisplayed()
    }

    @Test
    fun testRoomWithoutWidgetHasNoEntry() {
        compose.onNodeWithText("Echo").performClick()
        compose.onNodeWithContentDescription("Back").assertIsDisplayed()
        compose.onNodeWithTag("room-call").assertDoesNotExist()
        compose.onNodeWithTag("room-call-active").assertDoesNotExist()
    }

    private fun waitForTag(tag: String, unmerged: Boolean = false) {
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithTag(tag, useUnmergedTree = unmerged).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private companion object {
        const val WAIT_MS = 10_000L
    }
}
