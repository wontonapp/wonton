package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import app.wonton.ui.R
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 输入栏的停止钮（ADR-0061 第 10 节），只用 Debug 的假核心（发一句话就算 Bot 开了一轮，停的结果按那句话定，
 * 见 `ui_fake_agent_stop.rs`）：Bot 在跑时才有停止钮；点了先是在停、点不了，停成了说一声、按钮收走；
 * 被拒和没送到各有一行说明，按钮留着能再点。
 */
@RunWith(AndroidJUnit4::class)
class AgentStopTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun testStopAppearsWithATurnAndConfirmsTheStop() {
        enterRoom()
        compose.onNodeWithTag(STOP).assertDoesNotExist()

        send("hello")
        waitForTag(STOP)
        compose.onNodeWithTag(STOP).assertIsEnabled().performClick()
        compose.onNodeWithTag(STOP).assertIsNotEnabled()

        waitForTag("composer-stop-stopped")
        compose.onNodeWithTag(STOP).assertDoesNotExist()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithTag("composer-stop-stopped").fetchSemanticsNodes().isEmpty()
        }
    }

    @Test
    fun testRefusedStopSaysSoAndKeepsTheButton() {
        enterRoom()
        send("refuse")
        waitForTag(STOP)
        compose.onNodeWithTag(STOP).performClick()
        waitForTag("composer-stop-failed")
        compose.onNodeWithTag(STOP).assertIsEnabled()
    }

    @Test
    fun testUnreachableStopIsShownAsUncertain() {
        enterRoom()
        send("offline")
        waitForTag(STOP)
        compose.onNodeWithTag(STOP).performClick()
        waitForTag("composer-stop-uncertain")
        compose.onNodeWithTag("composer-stop-stopped").assertDoesNotExist()
        compose.onNodeWithTag(STOP).assertIsEnabled()
    }

    private fun enterRoom() {
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(WAIT_MS) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun send(text: String) {
        compose.onNode(hasSetTextAction()).performTextInput(text)
        compose.onNodeWithContentDescription(compose.activity.getString(R.string.composer_send)).performClick()
    }

    private fun waitForTag(tag: String) {
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() }
    }

    private companion object {
        const val STOP = "composer-stop"
        const val WAIT_MS = 10_000L
    }
}
