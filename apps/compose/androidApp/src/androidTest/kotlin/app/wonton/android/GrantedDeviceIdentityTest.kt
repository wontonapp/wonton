@file:Suppress("MagicNumber")

package app.wonton.android

import android.graphics.Bitmap
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.FontStyle
import app.wonton.protocol.PluginDeviceKind
import app.wonton.protocol.PluginGrantedDevice
import app.wonton.protocol.Rgb
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.ui.GrantedDevicesSection
import app.wonton.ui.design.WontonTheme
import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Native component state evidence only; this does not exercise host hardware authorization. */
@RunWith(AndroidJUnit4::class)
class GrantedDeviceIdentityTest {
    @get:Rule
    val compose = createComposeRule()

    private val a = PluginGrantedDevice(
        path = "/dev/serial/by-id/grant-a",
        name = "Device A",
        kind = PluginDeviceKind.SERIAL,
        system = false,
        available = true,
    )
    private val b = PluginGrantedDevice(
        path = "/dev/disk/by-id/grant-b",
        name = "Device B",
        kind = PluginDeviceKind.DISK,
        system = false,
        available = true,
    )
    private val c = PluginGrantedDevice(
        path = "/dev/bus/usb",
        name = null,
        kind = PluginDeviceKind.USB,
        system = false,
        available = true,
    )

    @Test
    fun reorderingAndRemovalKeepDisclosureWithItsGrant() {
        var shown by mutableStateOf(listOf(a, b, c))
        val appearance = previewAppearance()
        compose.setContent {
            WontonTheme(appearance) {
                GrantedDevicesSection(shown)
            }
        }
        assertOnlyVisible(null)
        details(a).performClick()
        assertOnlyVisible(a)

        compose.runOnIdle { shown = listOf(c, b, a) }
        assertOnlyVisible(a)
        screenshot("reordered-A-open")

        compose.runOnIdle { shown = listOf(c, b) }
        compose.onNodeWithTag(rowTag(a)).assertDoesNotExist()
        assertOnlyVisible(null)
        screenshot("removed-A-all-collapsed")

        details(b).performClick()
        assertOnlyVisible(b)
        compose.runOnIdle { shown = listOf(b) }
        assertOnlyVisible(b)
        screenshot("removed-other-keeps-B-open")

        compose.runOnIdle { shown = listOf(b.copy(name = "Renamed B", available = false)) }
        compose.onNodeWithText("Renamed B").assertIsDisplayed()
        assertOnlyVisible(b)
        screenshot("updated-metadata-keeps-B-open")
    }

    private fun details(device: PluginGrantedDevice) = compose.onNodeWithTag(
        "plugin-granted-device-details-${device.kind.name}-${device.path}",
    )

    private fun rowTag(device: PluginGrantedDevice) = "plugin-granted-device-${device.kind.name}-${device.path}"

    private fun assertOnlyVisible(expected: PluginGrantedDevice?) {
        listOf(a, b, c).forEach { device ->
            val path = compose.onNodeWithText(device.path, useUnmergedTree = true)
            if (expected != null && device.kind == expected.kind && device.path == expected.path) {
                path.assertIsDisplayed()
            } else {
                path.assertDoesNotExist()
            }
        }
    }

    private fun screenshot(state: String) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val bitmap = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "granted-device-identity")
            assertTrue("Screenshot directory unavailable", directory.isDirectory || directory.mkdirs())
            File(directory, "UI-FAKE-STATE-ONLY-$state.png").outputStream().use { output ->
                assertTrue("Screenshot encoding failed", bitmap.compress(Bitmap.CompressFormat.PNG, 100, output))
            }
        } finally {
            bitmap.recycle()
        }
    }

    private fun rgb(hex: Int) = Rgb(
        (hex shr 16 and 0xFF).toUByte(),
        (hex shr 8 and 0xFF).toUByte(),
        (hex and 0xFF).toUByte(),
    )

    private fun palette() = ThemeColors(
        canvas = rgb(0xFFFFFF),
        surface = rgb(0xFFFFFF),
        surfaceSubtle = rgb(0xF2F2F2),
        textPrimary = rgb(0x000000),
        textSecondary = rgb(0x666666),
        separator = rgb(0xDDDDDD),
        primary = rgb(0x000000),
        onPrimary = rgb(0xFFFFFF),
        success = rgb(0x2E7D32),
        warning = rgb(0xF9A825),
        danger = rgb(0xC62828),
        accentFill = rgb(0xE3F2FD),
        accentInk = rgb(0x1565C0),
        accentInkOnFill = rgb(0x0D47A1),
        textOnAccentFill = rgb(0x0D47A1),
        accentFillBorder = rgb(0xBBDEFB),
    )

    private fun previewAppearance() = AppearanceViewModel(
        accent = rgb(0x3366FF),
        presets = listOf(rgb(0x3366FF)),
        moreColors = emptyList(),
        swatchColumns = 6u,
        themeMode = ThemeMode.SYSTEM,
        themeModes = listOf(ThemeMode.SYSTEM),
        textSize = TextSize.STANDARD,
        textSizes = listOf(TextSize.STANDARD),
        font = FontStyle.DEFAULT,
        fonts = listOf(FontStyle.DEFAULT),
        light = palette(),
        dark = palette(),
        problem = null,
    )
}
