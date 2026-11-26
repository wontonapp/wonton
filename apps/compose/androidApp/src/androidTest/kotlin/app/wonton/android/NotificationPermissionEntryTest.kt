package app.wonton.android

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/// 设置 → 通知 → “系统通知”入口的权限分支：没允许时先弹系统的授权框，允许、拒绝都落到系统里云吞的通知页；
/// 允许以后直接去那一页，不再弹框。Debug 的假核心够撑起设置页，不用连家里的服务器。
///
/// 每条用例都从“还没问过权限”开始：androidTest 用了编排器并开了 clearPackageData（build.gradle），
/// 每条用例前清掉 App 数据，POST_NOTIFICATIONS 回到没问过的状态。
@RunWith(AndroidJUnit4::class)
class NotificationPermissionEntryTest {
    @get:Rule
    val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val ui get() = instrumentation.uiAutomation
    private val app get() = instrumentation.targetContext

    @Test
    fun denialStillLandsInTheSystemNotificationPage() {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU)
        answerStartupPrompt(allow = false)
        assertFalse(granted())
        openNotificationSettings()
        tapSystemNotificationsRowAndExpectSystemPage(allow = false)
    }

    @Test
    fun allowedEntryGoesStraightToTheSystemNotificationPage() {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU)
        answerStartupPrompt(allow = true)
        openNotificationSettings()
        tapSystemNotificationsRowAndExpectSystemPage(allow = true)
        assertTrue(granted())
    }

    /** 用例最后停在系统设置上：按返回一步回到 App，编排器接着清数据时 App 的任务还在，不会把下一条的进程一起收掉。 */
    @After
    fun backToTheApp() {
        ui.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
    }

    private fun granted(): Boolean =
        app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    /** App 一启动就要通知权限（PushRegistration）：等它弹出来，按 [allow] 答掉；已经给过时没有这一框。 */
    private fun answerStartupPrompt(allow: Boolean) {
        val shown = runCatching {
            waitForTop("系统的通知权限框") { it.endsWith(PERMISSION_CONTROLLER) }
        }.isSuccess
        if (!shown) return
        clickSystemButton(allow)
        waitForTop("回到 App") { it == app.packageName }
    }

    /** 侧边栏 → 设置 → 通知页。假核心只在 Debug 里，设置入口和普通开发包一样。 */
    private fun openNotificationSettings() {
        waitUntil { exists { compose.onNodeWithContentDescription("Sidebar") } }
        compose.onNodeWithContentDescription("Sidebar").performClick()
        waitUntil { exists { compose.onNodeWithTag("sidebar-settings") } }
        compose.onNodeWithTag("sidebar-settings").performClick()
        waitUntil { exists { compose.onNodeWithText("Notifications") } }
        compose.onNodeWithText("Notifications").performScrollTo().performClick()
        waitUntil { exists { compose.onNodeWithText("System notifications") } }
    }

    /** 点“系统通知”那一行：要么直接去系统页，要么先弹权限框（答掉再走）；最后都必须落到系统页。 */
    private fun tapSystemNotificationsRowAndExpectSystemPage(allow: Boolean) {
        compose.onNodeWithText("System notifications").performScrollTo().performClick()
        waitForTop("系统的通知权限框或系统设置页") { it.endsWith(PERMISSION_CONTROLLER) || it == SETTINGS }
        if (topPackage()?.endsWith(PERMISSION_CONTROLLER) == true) clickSystemButton(allow)
        waitForTop("系统的云吞通知页") { it == SETTINGS }
    }

    private fun exists(block: () -> Unit): Boolean = runCatching(block).isSuccess

    private fun waitUntil(timeoutMs: Long = WAIT_MS, what: String = "界面", block: () -> Boolean) {
        if (waitFor(timeoutMs, block)) return
        fail("等不到$what（当前前台 ${topPackage()}）")
    }

    private fun waitFor(timeoutMs: Long = WAIT_MS, block: () -> Boolean): Boolean {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            if (block()) return true
            Thread.sleep(200)
        }
        return block()
    }

    private fun waitForTop(what: String, timeoutMs: Long = WAIT_MS, check: (String) -> Boolean) {
        val reached = waitFor(timeoutMs) { topPackage()?.let(check) == true }
        if (!reached) fail("等不到$what（当前前台 ${topPackage()}）")
    }

    private fun topPackage(): String? = ui.rootInActiveWindow?.packageName?.toString()

    private fun clickSystemButton(allow: Boolean) {
        val labels = if (allow) listOf("Allow") else listOf("Don't allow", "Don’t allow", "Deny")
        val node = waitForNode(labels) ?: error("权限框里找不到${labels.joinToString(" / ")}")
        assertTrue("权限框的按钮点不动", node.performAction(AccessibilityNodeInfo.ACTION_CLICK))
    }

    private fun waitForNode(labels: List<String>): AccessibilityNodeInfo? {
        var found: AccessibilityNodeInfo? = null
        waitFor {
            val root = ui.rootInActiveWindow ?: return@waitFor false
            found = labels.firstNotNullOfOrNull { findLabel(root, it) }
            found != null
        }
        return found
    }

    private fun findLabel(node: AccessibilityNodeInfo, label: String): AccessibilityNodeInfo? =
        if (node.text?.toString() == label || node.contentDescription?.toString() == label) {
            node
        } else {
            (0 until node.childCount).firstNotNullOfOrNull { index ->
                node.getChild(index)?.let { child -> findLabel(child, label) }
            }
        }

    private companion object {
        const val WAIT_MS = 10_000L
        const val PERMISSION_CONTROLLER = "permissioncontroller"
        const val SETTINGS = "com.android.settings"
    }
}
