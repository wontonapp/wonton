package app.wonton.android

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.ui.R
import java.io.File
import java.io.FileOutputStream
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 依赖的界面走查（ADR-0053）：确认页列出一起装的提供方和它的权限、密钥；卸载提供方被在用的应用拦下，
 * 给出一起卸载或先停掉的选择。假数据里是 Autobangumi（要 torrent_client）和 qBittorrent。
 */
@RunWith(AndroidJUnit4::class)
class PluginDependencyTest {
    @get:Rule
    val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val strings = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun installPreviewListsTheProvider() {
        openStore()
        scroll().performScrollToNode(hasText("Autobangumi"))
        compose.onNodeWithText("Autobangumi").performClick()
        compose.onNodeWithText(installLabel()).performClick()
        compose.onNodeWithText("torrent_client").performScrollTo()
        compose.onNodeWithText("qBittorrent").performScrollTo()
        compose.onNodeWithText("Write to the downloads folder").performScrollTo()
        compose.onNodeWithText("API key").performScrollTo()
        compose.onNodeWithText("indexer").performScrollTo()
        compose.onNodeWithText(strings.getString(R.string.install_wants_connected_title)).performScrollTo()
        compose.onNodeWithText("Audio Book").performScrollTo()
        shot("install-provider")
    }

    @Test
    fun uninstallProviderShowsDependents() {
        compose.onNodeWithText("qBittorrent").performScrollTo().performClick()
        scroll().performScrollToNode(hasText(strings.getString(R.string.plugin_uninstall_action)))
        compose.onNodeWithText(strings.getString(R.string.plugin_uninstall_action)).performClick()
        compose.onNodeWithText("These apps still need qBittorrent").performScrollTo()
        compose.onNodeWithText("Autobangumi").performScrollTo()
        compose.onNodeWithText("Uninstall them too").performScrollTo()
        compose.onNodeWithText("Stop them first").performScrollTo()
        shot("uninstall-in-use")
    }

    @Test
    fun dependencyProblemOnTheAppPage() {
        compose.onNodeWithText("Autobangumi").performScrollTo().performClick()
        compose.onNodeWithText("qBittorrent isn't running, so this app can't use it right now.")
            .performScrollTo()
        compose.onNodeWithText("Start qBittorrent").performScrollTo()
        shot("dependency-problem")
    }

    private fun openStore() {
        compose.onNodeWithText(strings.getString(R.string.store_title)).performClick()
        compose.waitUntil(WAIT_MS) { compose.onAllNodes(hasText("Autobangumi")).fetchSemanticsNodes().isNotEmpty() }
    }

    /// 详情页的安装按钮：走查用的是英文环境，App 没有强制语言的假数据时按资源取。
    private fun installLabel(): String = strings.getString(R.string.store_get)

    private fun scroll() = compose.onAllNodes(hasScrollAction())[0]

    private fun shot(name: String) {
        val bitmap = compose.onRoot().captureToImage().asAndroidBitmap()
        val dir = InstrumentationRegistry.getInstrumentation().targetContext.getExternalFilesDir(null)
            ?: return
        val file = File(dir, "$name.png")
        FileOutputStream(file).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply {
            putString("shot", file.absolutePath)
        })
    }

    private companion object {
        const val WAIT_MS = 10_000L
    }
}
