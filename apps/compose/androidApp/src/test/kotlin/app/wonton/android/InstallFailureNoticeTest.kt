package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.PluginFailure
import app.wonton.ui.InstallFailure
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.problem
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * 安装完成页失败时按 PluginFailure 的类别出标题和下一步（#664）：标题用商店任务行的
 * “安装失败：原因”格式，类别文字取同一套资源，不落到通用文案。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class InstallFailureNoticeTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun portTakenShowsCategoryTitleAndNextStep() {
        assertCategory(PluginFailure.LAN_PORT_TAKEN)
    }

    @Test
    fun changedShowsCategoryTitleAndNextStep() {
        assertCategory(PluginFailure.CHANGED)
    }

    private fun assertCategory(reason: PluginFailure) {
        val context = ApplicationProvider.getApplicationContext<Application>()
        val message = "核心写的技术细节"
        val problem = reason.problem(message)
        compose.setContent {
            WontonTheme(testAppearance()) {
                InstallFailure(reason, message, onReview = {})
            }
        }

        compose.onNodeWithText(
            context.getString(R.string.plugin_task_install_failed, context.getString(problem.titleRes!!)),
        ).assertIsDisplayed()
        compose.onNodeWithText(context.getString(problem.nextStepRes!!)).assertIsDisplayed()
    }
}
