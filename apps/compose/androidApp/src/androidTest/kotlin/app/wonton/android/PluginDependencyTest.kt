package app.wonton.android

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
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
import android.os.Build
import androidx.test.rule.GrantPermissionRule
import org.junit.Rule
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.Before
import org.junit.Test
import org.junit.Assert.assertTrue
import org.junit.runner.RunWith

/**
 * 依赖的界面走查（ADR-0053）：确认页列出一起装的提供方和它的权限、密钥；卸载提供方被在用的应用拦下，
 * 给出一起卸载或先停掉的选择。假数据里是 Autobangumi（要 download_client）和 qBittorrent。
 */
@RunWith(AndroidJUnit4::class)
class PluginDependencyTest {
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(android.Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    private val strings = InstrumentationRegistry.getInstrumentation().targetContext

    @Before
    fun backToTheRoot() {
        // 上次留下的页面（如备份页）会盖住抽屉：还在子页就按返回，回到有侧边栏按钮的根页。
        repeat(4) {
            val drawer = compose.onAllNodes(
                hasContentDescription(strings.getString(R.string.sidebar_open)),
                useUnmergedTree = true,
            ).fetchSemanticsNodes()
            if (drawer.isNotEmpty()) return@repeat
            runCatching {
                compose.activityRule.scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }
                compose.waitForIdle()
            }
        }
    }

    @Test
    fun installPreviewListsTheProvider() {
        openStore()
        scroll().performScrollToNode(hasText("Autobangumi"))
        compose.onNodeWithText("Autobangumi", useUnmergedTree = true).performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasText("Follows the season and grabs new episodes"), useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        // 商店列表每行也有“Get”：详情页的按钮在最后。
        val gets = compose.onAllNodes(hasText(installLabel()), useUnmergedTree = true)
        gets[gets.fetchSemanticsNodes().size - 1].performClick()
        compose.onNodeWithText("download_client", useUnmergedTree = true).performScrollTo()
        compose.onAllNodesWithText("qBittorrent", useUnmergedTree = true, substring = true)[0].performScrollTo()
        compose.onNodeWithText("Write to the downloads folder", useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText("API key", useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText("indexer", useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText(strings.getString(R.string.install_wants_connected_title), useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText("Audio Book", useUnmergedTree = true).performScrollTo()
        // 随批装的提供方：只列出来，开关在单独装提供方时才有。
        assertTrue(
            compose.onAllNodes(hasContentDescription("Audio Book"), useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
        )
        shot("install-provider")
    }

    @Test
    fun installingTheProviderOffersToConnectIt() {
        openStore()
        scroll().performScrollToNode(hasText("qBittorrent"))
        compose.onNodeWithText("qBittorrent", useUnmergedTree = true).performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasText("The household's downloads"), useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        val gets = compose.onAllNodes(hasText(installLabel()), useUnmergedTree = true)
        gets[gets.fetchSemanticsNodes().size - 1].performClick()
        // 直接装提供方时它的预览没有 install_providers（不是缺能力），只有想用它的应用一节。
        compose.onNodeWithText(strings.getString(R.string.install_wants_connected_title), useUnmergedTree = true)
            .performScrollTo()
        compose.onNodeWithText("Audio Book", useUnmergedTree = true).performScrollTo()
        val switch = compose.onNodeWithContentDescription("Audio Book", useUnmergedTree = true)
        switch.assertIsOff()
        switch.performClick()
        switch.assertIsOn()
        shot("install-provider-direct")
    }

    @Test
    fun uninstallProviderShowsDependents() {
        openApps()
        compose.onNodeWithText("qBittorrent", useUnmergedTree = true).performScrollTo().performClick()
        scroll().performScrollToNode(hasText(strings.getString(R.string.plugin_uninstall_action)))
        compose.onNodeWithText(strings.getString(R.string.plugin_uninstall_action), useUnmergedTree = true).performClick()
        compose.onNodeWithText("These apps still need qBittorrent", useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText("Autobangumi", useUnmergedTree = true).performScrollTo()
        compose.onNodeWithText("Uninstall them too", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithText("Stop them first", useUnmergedTree = true).assertIsDisplayed()
        shot("uninstall-in-use")
    }

    @Test
    fun dependencyProblemOnTheAppPage() {
        openApps()
        compose.onNodeWithText("Autobangumi", useUnmergedTree = true).performScrollTo().performClick()
        compose.onAllNodesWithText("qBittorrent isn’t running", useUnmergedTree = true, substring = true)[0]
            .performScrollTo()
        compose.onNodeWithText("Start qBittorrent", useUnmergedTree = true).performScrollTo().assertIsDisplayed()
        shot("dependency-problem")
    }

    private fun openDrawer() {
        compose.onNodeWithContentDescription(strings.getString(R.string.sidebar_open)).performClick()
    }

    private fun openApps() {
        openDrawer()
        compose.onAllNodesWithText(strings.getString(R.string.sidebar_all), useUnmergedTree = true)[0].performClick()
    }

    private fun openStore() {
        openDrawer()
        compose.onAllNodesWithText(strings.getString(R.string.sidebar_store), useUnmergedTree = true)[0].performClick()
        compose.waitUntil(WAIT_MS) { compose.onAllNodes(hasText("Autobangumi")).fetchSemanticsNodes().isNotEmpty() }
    }

    /// 详情页的安装按钮：走查用的是英文环境，App 没有强制语言的假数据时按资源取。
    private fun installLabel(): String = strings.getString(R.string.store_get)

    /// 抽屉还开着时先命中抽屉自己的滚动容器，取最外层那一页的。
    private fun scroll(): SemanticsNodeInteraction {
        val all = compose.onAllNodes(hasScrollAction(), useUnmergedTree = true)
        return all[all.fetchSemanticsNodes().size - 1]
    }

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
