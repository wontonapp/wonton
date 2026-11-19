package app.wonton.android

import android.Manifest
import android.os.Build
import android.view.KeyEvent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.ui.PageExitHintPrefs
import app.wonton.ui.R
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 整页应用的返回引导（ADR-0095）：第一次打开整页时出现，点了“知道了”消失，再进不出现。
 * 用 Debug 假核心里 Family 房间的通话页当整页；别的界面测试启动时都当作看过，这里先改回没看过。
 */
@RunWith(AndroidJUnit4::class)
class PageExitHintTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    private val prefs get() = PageExitHintPrefs.of(compose.activity)

    @Before
    fun forgetTheHint() = compose.runOnUiThread { prefs.seen = false }

    @After
    fun skipTheHintForOtherTests() = compose.runOnUiThread { prefs.seen = true }

    @Test
    fun testHintShowsOnceUntilAcknowledged() {
        openCallPage()
        waitForTag(HINT)
        compose.onNodeWithText(compose.activity.getString(R.string.tab_exit_hint)).assertIsDisplayed()

        // 用返回离开不算看过：下次打开还教。
        pressBack()
        openCallPage()
        waitForTag(HINT)

        compose.onNodeWithText(compose.activity.getString(R.string.tab_exit_hint_got_it)).performClick()
        compose.onNodeWithTag(HINT).assertDoesNotExist()

        pressBack()
        openCallPage()
        waitForTag("page-hint")
        compose.onNodeWithTag(HINT).assertDoesNotExist()
        pressBack()
    }

    private fun openCallPage() {
        if (compose.onAllNodesWithTag("room-call-active").fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText("Family").performClick()
        }
        waitForTag("room-call-active")
        compose.onNodeWithTag("room-call-active").performClick()
    }

    private fun pressBack() {
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        waitForTag("room-call-active")
    }

    private fun waitForTag(tag: String) {
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() }
    }

    private companion object {
        const val HINT = "page-exit-hint"
        const val WAIT_MS = 10_000L
    }
}
