package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import app.wonton.ui.R
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 安装卡片上选设备（ADR-0036），只用 Debug 的假核心（`ui_fake_devices.rs` 里“等你确认”的那张卡片）：串口没选时不能同意；
 * 系统盘有标记、硬盘默认不勾；同意以后发回去的是选中的串口和勾上的盘。假核心把收到的设备 ID 写进卡片的结果，这里照着看。
 */
@RunWith(AndroidJUnit4::class)
class DeviceGrantTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun testChosenDevicesGoBackWithTheAnswer() {
        compose.onNodeWithText(compose.activity.getString(R.string.confirmations_title)).performClick()
        compose.onNodeWithText(CARD_TITLE).performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasTestTag("device-$SERIAL")).fetchSemanticsNodes().isNotEmpty()
        }

        compose.onNodeWithTag("device-usb-all").assertIsDisplayed()
        val systemTag = hasText(compose.activity.getString(R.string.install_device_system_disk))
        compose.onNode(systemTag and hasAnyAncestor(hasTestTag("device-$SYSTEM_DISK"))).assertIsDisplayed()
        compose.onNode(systemTag and hasAnyAncestor(hasTestTag("device-$DATA_DISK"))).assertDoesNotExist()
        option(SYSTEM_DISK, isToggleable()).assertIsOff()
        option(DATA_DISK, isToggleable()).assertIsOff()
        compose.onNodeWithText(INSTALL).performScrollTo().assertIsNotEnabled()

        option(SERIAL, isSelectable()).performScrollTo().performClick()
        option(DATA_DISK, isToggleable()).performScrollTo().performClick()
        option(SERIAL, isSelectable()).assertIsSelected()
        option(DATA_DISK, isToggleable()).assertIsOn()
        option(SYSTEM_DISK, isToggleable()).assertIsOff()

        compose.onNodeWithText(INSTALL).performScrollTo().assertIsEnabled().performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasText("$SERIAL\n$DATA_DISK")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(SYSTEM_DISK, substring = true).assertDoesNotExist()
    }

    /** 一项设备那一行里能点的选项：串口是单选，硬盘是勾选。 */
    private fun option(device: String, kind: SemanticsMatcher) =
        compose.onNode(kind and hasAnyAncestor(hasTestTag("device-$device")))

    private companion object {
        const val WAIT_MS = 10_000L
        const val CARD_TITLE = "Echo wants to install Home Hub"
        const val INSTALL = "Install"
        const val SERIAL = "serial:0:/dev/serial/by-id/usb-ITead_Sonoff_Zigbee_3.0-if00-port0"
        const val SYSTEM_DISK = "disk:/dev/disk/by-id/nvme-Samsung_SSD_990_PRO_1TB"
        const val DATA_DISK = "disk:/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0"
    }
}
