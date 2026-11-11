package app.wonton.android

import android.Manifest
import android.content.ContentValues
import android.graphics.Bitmap
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
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
}
