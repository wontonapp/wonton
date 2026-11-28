package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.rule.GrantPermissionRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.Action
import app.wonton.protocol.BarEntry
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import org.junit.Rule
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 覆盖根页面来回切换导致的页面组合切换；每次都确认目标页面实际渲染，再检查房间输入仍可用。
 * 这是 onReuse/movableContent 路径的行为回归，帧级竞态不保证在旧实现上稳定失败。
 */
@RunWith(AndroidJUnit4::class)
class PageFlipMovableContentTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun chatPageSurvivesRepeatedPageFlips() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.SelectBarItem(BarEntry.Agent))))
        assertPageFixture("Echo")
        repeat(FLIPS) {
            assertNull(core.send(Action.Pages(PageAction.SelectBarItem(BarEntry.Apps))))
            assertPageFixture("Notes")
            assertNull(core.send(Action.Pages(PageAction.SelectBarItem(BarEntry.Agent))))
            assertPageFixture("Echo")
        }
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1
        }
        compose.onNode(hasSetTextAction()).assertIsDisplayed()
    }

    private fun assertPageFixture(name: String) {
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(name).assertIsDisplayed()
    }

    private companion object {
        const val WAIT_MS = 20_000L
        const val FLIPS = 40
    }
}
