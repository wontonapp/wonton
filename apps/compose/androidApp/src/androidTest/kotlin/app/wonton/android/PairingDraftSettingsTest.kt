package app.wonton.android

import android.Manifest
import android.content.Context
import android.os.Build
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
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
import app.wonton.ui.send
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/// 设置→服务器→添加服务器那页（`AddServerPairing`）的配对页：按返回退一层、不退出 App，
/// 打了一半的设备名进草稿，重新进这页还填着。
///
/// 改前这一页自己的 `BackHandler` 把上层的返回吞了，按一下整页关掉，输入跟着没。
/// 被测的 App 没配对，设置里没有服务器项；设置里那一路开的就是 `AddServerPairing`，
/// 这里直接把 `AddServer` 发给核心打开同一页，不去依赖设置页的服务器行。
@RunWith(AndroidJUnit4::class)
class PairingDraftSettingsTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    /// 通知和相机权限弹窗都会把被测 Activity 顶到后台，先发掉再启动。
    private val permissions: TestRule =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.CAMERA)
        } else {
            GrantPermissionRule.grant(Manifest.permission.CAMERA)
        }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun backFromAddServerKeepsDraft() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        // 进程里先跑过真核心的类时核心换不过来：真核心没配对、核心拒绝“添加服务器”，这一页打不开。
        assumeTrue("进程里先起了真核心，假核心没生效", core.servers().servers.isNotEmpty())
        core.send(Action.Home(HomeAction.AddServer))
        // 首屏本来也是配对页（同一批输入框），要等 adding 真的翻过来，才算进的是“添加服务器”这一页。
        compose.waitUntil(timeoutMillis = 5_000) { core.servers().adding }

        nameField().performScrollTo().performTextReplacement("kitchen-ipad")
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

        // 返回上一层：添加页收掉，不退出 App，输入落到草稿里。
        compose.waitUntil(timeoutMillis = 5_000) { !core.servers().adding }
        assertFalse(compose.activity.isFinishing)
        assertEquals("kitchen-ipad", prefs().getString("device_name", null))

        // 再进添加页，打过的名字还填着。
        core.send(Action.Home(HomeAction.AddServer))
        compose.waitUntil(timeoutMillis = 5_000) { core.servers().adding }
        awaitPairingForm()
        nameField().performScrollTo().assert(hasText("kitchen-ipad"))
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
