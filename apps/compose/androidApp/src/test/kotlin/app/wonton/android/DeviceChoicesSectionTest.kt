package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.client.DeviceChoices
import app.wonton.client.SerialChoice
import app.wonton.protocol.PluginDevice
import app.wonton.protocol.PluginDeviceKind
import app.wonton.ui.DeviceChoicesSection
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * “设备”一节只照核心整理好的结果画（ADR-0036）：系统盘有标记，点一项把它的 ID 交回去，原始 ID 收在“详情”里，
 * 卡片正在发送时点不动。分组和点选规则在核心的行为测试里（`device_choices_tests.rs`）。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], application = Application::class, qualifiers = "w390dp-h844dp-mdpi")
class DeviceChoicesSectionTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<Application>()
    private val serial = device("serial:0:/dev/serial/by-id/usb-zigbee", "Sonoff Zigbee 3.0", PluginDeviceKind.SERIAL)
    private val systemDisk = device("disk:/dev/disk/by-id/nvme-system", "Samsung SSD 990", PluginDeviceKind.DISK, true)
    private val dataDisk = device("disk:/dev/disk/by-id/ata-data", "WDC WD40EFRX", PluginDeviceKind.DISK)

    private fun device(id: String, name: String, kind: PluginDeviceKind, system: Boolean = false) = PluginDevice(
        id = id,
        name = name,
        target = if (kind == PluginDeviceKind.SERIAL) "/dev/ttyZIGBEE" else "",
        service = if (kind == PluginDeviceKind.SERIAL) "zigbee2mqtt" else "smart",
        optional = false,
        kind = kind,
        system = system,
        chosen = false,
    )

    private val choices = DeviceChoices(
        serial = listOf(SerialChoice("zigbee2mqtt", optional = false, devices = listOf(serial))),
        serialMissing = false,
        usb = true,
        disks = listOf(systemDisk, dataDisk),
        disksMissing = false,
        unknown = emptyList(),
        ready = false,
    )

    @Test
    fun systemDiskIsMarkedAndATapHandsBackTheDeviceId() {
        val toggled = mutableListOf<String>()
        compose.setContent {
            WontonTheme(testAppearance()) { DeviceChoicesSection(choices) { toggled += it } }
        }
        val systemTag = hasText(context.getString(R.string.install_device_system_disk))
        compose.onNode(systemTag and hasAnyAncestor(hasTestTag("device-${systemDisk.id}"))).assertIsDisplayed()
        compose.onNode(systemTag and hasAnyAncestor(hasTestTag("device-${dataDisk.id}"))).assertDoesNotExist()
        compose.onNodeWithTag("device-usb-all").assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.install_device_serial_for, "zigbee2mqtt")).assertIsDisplayed()

        compose.onNodeWithText(dataDisk.name).performClick()
        compose.onNodeWithText(serial.name).performClick()
        assertEquals(listOf(dataDisk.id, serial.id), toggled)

        compose.onNodeWithText(dataDisk.id, substring = true).assertDoesNotExist()
        compose.onNodeWithText(context.getString(R.string.common_details)).performClick()
        compose.onNodeWithText(dataDisk.id, substring = true).assertIsDisplayed()
    }

    @Test
    fun aSectionThatIsBeingSentCannotBeChanged() {
        val toggled = mutableListOf<String>()
        compose.setContent {
            WontonTheme(testAppearance()) { DeviceChoicesSection(choices, enabled = false) { toggled += it } }
        }
        compose.onNodeWithText(dataDisk.name).performClick()
        assertEquals(emptyList<String>(), toggled)
    }
}
