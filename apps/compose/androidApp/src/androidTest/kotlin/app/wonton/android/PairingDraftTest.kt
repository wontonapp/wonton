package app.wonton.android

import android.Manifest
import android.content.Context
import android.os.Build
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onLast
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.HomeAction
import app.wonton.protocol.HomeStatus
import app.wonton.ui.send
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement

/// 首屏配对页按返回就是退出 App（系统惯例），打了一半的设备名不能跟着没：退出前存进草稿，重开还填着。
///
/// 真核心（`MainActivity`，全新安装 = 没配对）：首屏就是 `HomeRoute` 的根配对页，返回交给系统退出。
/// 设置里那个“添加服务器”页（`AddServerPairing`）另有 `PairingDraftSettingsTest` 按假核心验。
@RunWith(AndroidJUnit4::class)
class PairingDraftTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    /// 通知和相机权限弹窗都会把被测 Activity 顶到后台，先发掉再启动。
    private val permissions: TestRule =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.CAMERA)
        } else {
            GrantPermissionRule.grant(Manifest.permission.CAMERA)
        }

    /// 每个用例都从干净的草稿开始：上一个类留下的名字不能影响这条。
    private val cleanDraft = object : TestRule {
        override fun apply(base: Statement, description: Description) =
            object : Statement() {
                override fun evaluate() {
                    prefs().edit().clear().apply()
                    base.evaluate()
                }
            }
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(cleanDraft).around(compose)

    @Test
    fun typedDeviceNameSurvivesExit() {
        val rootScreen = openPairingPage()
        nameField().performScrollTo().performTextReplacement("kitchen-ipad")
        assertEquals("kitchen-ipad", prefs().getString("device_name", null))

        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

        // 根配对页交给系统退出；草稿在离开前就落了。
        if (rootScreen) assertTrue(compose.activity.isFinishing) else assertFalse(compose.activity.isFinishing)
        assertEquals("kitchen-ipad", prefs().getString("device_name", null))
    }

    /// 重开这一页时把草稿里的设备名填回来（改前那棵树只读默认名）。
    @Test
    fun savedDeviceNameIsFilledBack() {
        openPairingPage()
        prefs().edit().putString("device_name", "kitchen-ipad").apply()
        compose.activityRule.scenario.recreate()
        nameField().performScrollTo().assert(hasText("kitchen-ipad"))
    }

    /// 打开配对页。真核心没配对时首屏就是它；进程里先起过假核心时核心换不过来（`core` 一个进程一个），
    /// 首屏在假核心里是应用列表、没有输入框，这时退回“添加服务器”页。返回 true 表示是首屏根页。
    private fun openPairingPage(): Boolean {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val rootScreen = core.home().status is HomeStatus.NotConfigured
        if (!rootScreen) {
            core.send(Action.Home(HomeAction.AddServer))
            compose.waitUntil(timeoutMillis = 5_000) { core.servers().adding }
        }
        awaitPairingForm()
        return rootScreen
    }

    /// 别把草稿留给后面的类：别的用例往空的码框里打字，残留会接在前面。
    @After
    fun clearDraft() {
        prefs().edit().clear().apply()
    }

    /// 设备名字段的 tag 是这次新加的；改前那棵树只有两个文本框（码、设备名），按“最后一个可输入框”找。
    private fun nameField() =
        if (compose.onAllNodesWithTag("pairing-device-name").fetchSemanticsNodes().isNotEmpty()) {
            compose.onNodeWithTag("pairing-device-name")
        } else {
            compose.onAllNodes(hasSetTextAction()).onLast()
        }

    private fun awaitPairingForm() {
        compose.waitUntil(timeoutMillis = 60_000) {
            runCatching {
                compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().isNotEmpty()
            }.getOrDefault(false)
        }
    }

    private fun prefs() = InstrumentationRegistry.getInstrumentation().targetContext
        .getSharedPreferences("pairing_draft", Context.MODE_PRIVATE)
}
