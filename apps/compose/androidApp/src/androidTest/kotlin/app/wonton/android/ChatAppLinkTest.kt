package app.wonton.android

import android.Manifest
import android.app.Instrumentation
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.view.KeyEvent
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToKey
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.rules.RuleChain
import org.junit.Test
import org.junit.runner.RunWith

/** 消息里的 `wonton:` 链接就地打开（ADR-0041），不再绕系统一圈；认不得的提示一句。 */
@RunWith(AndroidJUnit4::class)
class ChatAppLinkTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules = RuleChain.outerRule(permissions).around(compose)

    @Before
    fun openRoom() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    @Test
    fun wontonLinkOpensInPlace() {
        val filter = IntentFilter(Intent.ACTION_VIEW).apply { addDataScheme("wonton") }
        val monitor = Instrumentation.ActivityMonitor(filter, null, false)
        InstrumentationRegistry.getInstrumentation().addMonitor(monitor)
        try {
            scrollTo(6)
            compose.onNodeWithText("wonton://settings/peers/?from=chat", substring = true, useUnmergedTree = true)
                .performClick()
            compose.waitUntil(10_000) {
                compose.onAllNodesWithText("Other servers").fetchSemanticsNodes().isNotEmpty()
            }
            compose.onNodeWithContentDescription("Add").assertIsDisplayed()
            assertEquals("打开 wonton:// 链接绕了一圈", 0, monitor.hits)
            pressBack()
        } finally {
            InstrumentationRegistry.getInstrumentation().removeMonitor(monitor)
        }
    }

    @Test
    fun unknownWontonLinkShowsMessage() {
        val filter = IntentFilter(Intent.ACTION_VIEW).apply { addDataScheme("wonton") }
        val monitor = Instrumentation.ActivityMonitor(filter, null, false)
        InstrumentationRegistry.getInstrumentation().addMonitor(monitor)
        try {
            scrollTo(7)
            compose.onNodeWithText("wonton:tab/from-the-future", substring = true, useUnmergedTree = true)
                .performClick()
            compose.onNodeWithText("Update the app to open this link").assertIsDisplayed()
            assertEquals(0, monitor.hits)
        } finally {
            InstrumentationRegistry.getInstrumentation().removeMonitor(monitor)
        }
    }

    private fun scrollTo(index: Int) {
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            }, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
    }

    private fun pressBack() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
    }
}
