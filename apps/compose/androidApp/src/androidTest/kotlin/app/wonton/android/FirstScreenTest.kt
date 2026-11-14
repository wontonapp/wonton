package app.wonton.android

import android.Manifest
import android.content.ContentValues
import android.graphics.Bitmap
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.PairingFailure
import app.wonton.protocol.PairingStatus
import app.wonton.ui.R as UiR
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/// 冷启动落在首屏：全新安装是配对页。夜跑用它证明 App 在模拟器上装得上、起得来、首屏画得出来。
/// 不依赖服务端。
@RunWith(AndroidJUnit4::class)
class FirstScreenTest {
    private val compose = createAndroidComposeRule<MainActivity>()

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
    fun freshInstallShowsPairingScreen() {
        // 首帧还没画出来时没有 Compose 层级，取语义树会抛 IllegalStateException；等它出现。
        compose.waitUntil(timeoutMillis = 60_000) {
            runCatching { compose.onAllNodesWithTag("pairing-submit").fetchSemanticsNodes().isNotEmpty() }
                .getOrDefault(false)
        }
        compose.waitForIdle()
        compose.onNodeWithTag("pairing-code").assertExists()
        compose.onNodeWithTag("pairing-submit").performScrollTo().assertIsDisplayed()

        // 测试结束会卸载 App，自己的 filesDir 跟着没了：截图放进系统相册，由工作流 pull 成产物。
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "first-screen.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/WontonUITests")
        }
        val uri = context.contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
            ?: return
        context.contentResolver.openOutputStream(uri)?.use { out ->
            compose.onRoot().captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, out)
        }
    }

    @Test
    fun unpairedSidebarEntriesKeepPairingAndRetryAvailable() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        compose.waitUntil(timeoutMillis = 10_000) {
            runCatching { compose.onAllNodesWithTag("pairing-code").fetchSemanticsNodes().isNotEmpty() }
                .getOrDefault(false)
        }
        assertNull(core.home().serverId)
        assertTrue(core.home().status is HomeStatus.NotConfigured)
        checkSidebarEntries("unpaired") {
            compose.onNodeWithTag("pairing-code").assertExists()
            compose.onNodeWithTag("pairing-submit").performScrollTo().assertIsDisplayed()
        }
        val typedInput = "restore unsent input"
        compose.onNodeWithTag("pairing-code").performScrollTo().performTextInput(typedInput)
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("pairing-code").performScrollTo().assertTextEquals(typedInput).performTextClearance()

        val code = "wonton://pair?v=0&id=d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a" +
            "&addr=127.0.0.1%3A9&domain=home.example&core=http%3A%2F%2Fhome.example%2F" +
            "&secret=abababababababababababababababab"
        compose.onNodeWithTag("pairing-code").performScrollTo().performTextInput(code)
        compose.onNodeWithTag("pairing-submit").performScrollTo().assertIsEnabled().performClick()
        compose.waitUntil(timeoutMillis = 90_000) { core.home().pairing is PairingStatus.Failed }
        val failure = core.home().pairing as PairingStatus.Failed
        assertEquals(PairingFailure.TRANSPORT_UNAVAILABLE, failure.reason)
        assertNull(core.home().serverId)
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("pairing-retry").performScrollTo().assertIsDisplayed().assertIsEnabled().performClick()
        compose.waitUntil(timeoutMillis = 10_000) { core.home().pairing is PairingStatus.InProgress }
        compose.waitUntil(timeoutMillis = 90_000) { core.home().pairing is PairingStatus.Failed }
        assertEquals(PairingFailure.TRANSPORT_UNAVAILABLE, (core.home().pairing as PairingStatus.Failed).reason)
        checkSidebarEntries("unreachable") {
            compose.onNodeWithTag("pairing-retry").performScrollTo().assertIsDisplayed().assertIsEnabled()
            assertEquals(PairingFailure.TRANSPORT_UNAVAILABLE, (core.home().pairing as PairingStatus.Failed).reason)
        }
    }

    private fun checkSidebarEntries(suffix: String, verify: () -> Unit) {
        val destinations = listOf(UiR.string.sidebar_chats to "chat", UiR.string.sidebar_new_chat to "new-chat")
        destinations.forEach { (label, name) ->
            compose.onNodeWithTag("sidebar-open").performClick()
            compose.onNodeWithText(compose.activity.getString(label)).performClick()
            compose.waitForIdle()
            compose.onAllNodes(isDialog()).assertCountEquals(0)
            verify()
            saveScreen("$suffix-$name")
        }
    }

    private fun saveScreen(name: String) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "offline-entry-$name.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/WontonUITests")
        }
        val uri = requireNotNull(context.contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values))
        requireNotNull(context.contentResolver.openOutputStream(uri)).use { output ->
            val image = compose.onRoot().captureToImage().asAndroidBitmap()
            assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, output))
        }
    }
}
