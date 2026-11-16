package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.PluginDeviceKind
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/** UI-FAKE LAYOUT ONLY: real navigation and native rendering, not host hardware authorization E2E. */
@RunWith(AndroidJUnit4::class)
class GrantedDeviceDetailTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun grantedDevicesRetainMissingDeviceAndRevealOnlyExpandedPath() {
        openHomeHubDetail()
        val names = listOf(
            "Sonoff Zigbee 3.0",
            compose.activity.getString(R.string.install_device_usb_all),
            "Samsung SSD 990 PRO 1TB",
            "WDC WD40EFRX 4TB",
            compose.activity.getString(R.string.plugin_device_other),
        )
        names.forEachIndexed { index, name ->
            rowText(index, name).performScrollTo().assertIsDisplayed()
            assertState(index, R.string.install_device_system_disk, index == 2)
            assertState(index, R.string.plugin_device_unavailable, index == 0)
        }
        PATHS.forEach { compose.onNodeWithText(it, useUnmergedTree = true).assertDoesNotExist() }
        rowText(0, names[0]).performScrollTo().assertIsDisplayed()
        screenshot("list")
        rowText(0, compose.activity.getString(R.string.plugin_device_unavailable)).assertIsDisplayed()
        screenshot("unavailable")
        compose.onNodeWithTag("plugin-granted-device-details-${identity(0)}")
            .performScrollTo().performClick()
        compose.onNodeWithText(PATHS[0], useUnmergedTree = true).performScrollTo().assertIsDisplayed()
        PATHS.drop(1).forEach { compose.onNodeWithText(it, useUnmergedTree = true).assertDoesNotExist() }
        screenshot("expanded-path")
    }

    private fun openHomeHubDetail() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasTestTag("sidebar-open")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("sidebar-open").performClick()
        compose.onNodeWithTag("sidebar-all-apps").performClick()
        compose.onNodeWithText("Home Hub").performScrollTo().performTouchInput { longClick() }
        compose.onNodeWithText(compose.activity.getString(R.string.common_details)).performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasTestTag("plugin-granted-devices")).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun rowText(index: Int, value: String) = compose.onNode(
        hasText(value) and hasAnyAncestor(hasTestTag("plugin-granted-device-${identity(index)}")),
        useUnmergedTree = true,
    )

    private fun identity(index: Int) = "${KINDS[index].name}-${PATHS[index]}"

    private fun assertState(index: Int, resource: Int, expected: Boolean) {
        val node = rowText(index, compose.activity.getString(resource))
        if (expected) node.assertIsDisplayed() else node.assertDoesNotExist()
    }

    private fun screenshot(state: String) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val bitmap = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            val directory = File(compose.activity.getExternalFilesDir(null), "granted-device-detail")
            assertTrue("Screenshot directory unavailable", directory.isDirectory || directory.mkdirs())
            File(directory, "UI-FAKE-LAYOUT-ONLY-$state.png").outputStream().use { output ->
                assertTrue("Screenshot encoding failed", bitmap.compress(Bitmap.CompressFormat.PNG, 100, output))
            }
        } finally {
            bitmap.recycle()
        }
    }

    private companion object {
        const val WAIT_MS = 20_000L
        val KINDS = listOf(
            PluginDeviceKind.SERIAL,
            PluginDeviceKind.USB,
            PluginDeviceKind.DISK,
            PluginDeviceKind.DISK,
            PluginDeviceKind.OTHER,
        )
        val PATHS = listOf(
            "/dev/serial/by-id/usb-ITead_Sonoff_Zigbee_3.0-if00-port0",
            "/dev/bus/usb",
            "/dev/disk/by-id/nvme-Samsung_SSD_990_PRO_1TB",
            "/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0",
            "/dev/example-hardware",
        )
    }
}
