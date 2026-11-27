package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/** 冒烟：真核心的全新安装起得来、落在配对页。不依赖服务端。 */
@RunWith(AndroidJUnit4::class)
class FirstScreenTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    /** 通知和相机权限弹窗都会把被测 Activity 顶到后台，先发掉再启动。 */
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
            runCatching { compose.onAllNodesWithTag("pairing-code").fetchSemanticsNodes().isNotEmpty() }
                .getOrDefault(false)
        }
    }
}
