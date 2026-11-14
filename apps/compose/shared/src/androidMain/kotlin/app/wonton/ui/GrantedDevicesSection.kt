package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.PluginDeviceKind
import app.wonton.protocol.PluginGrantedDevice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Device
import app.wonton.ui.icons.Disk
import app.wonton.ui.icons.SerialPort
import app.wonton.ui.icons.Usb
import app.wonton.ui.icons.WontonIcon

/** Installed grants are supplied by the core, including devices that are no longer available. */
@Composable
fun GrantedDevicesSection(devices: List<PluginGrantedDevice>) {
    DetailSection(stringResource(R.string.plugin_granted_devices)) {
        Column(
            Modifier.fillMaxWidth().testTag("plugin-granted-devices"),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            devices.forEachIndexed { index, device ->
                GrantedDeviceRow(device, index)
            }
        }
    }
}

@Composable
private fun GrantedDeviceRow(device: PluginGrantedDevice, index: Int) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().testTag("plugin-granted-device-$index"),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(
            grantedDeviceIcon(device.kind),
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Column(
            Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Column(
                Modifier.semantics(mergeDescendants = true) {},
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            ) {
                Text(
                    device.name?.takeIf { it.isNotEmpty() } ?: grantedDeviceLabel(device.kind),
                    style = WontonTheme.typography.body,
                    color = if (device.available) colors.textPrimary.toColor() else colors.textSecondary.toColor(),
                )
                if (!device.available) Tag(stringResource(R.string.plugin_device_unavailable), colors.warning)
                if (device.system) Tag(stringResource(R.string.install_device_system_disk), colors.danger)
            }
            ProblemDetail(device.path, modifier = Modifier.testTag("plugin-granted-device-details-$index"))
        }
    }
}

@Composable
private fun grantedDeviceLabel(kind: PluginDeviceKind): String = stringResource(
    when (kind) {
        PluginDeviceKind.SERIAL -> R.string.plugin_device_serial
        PluginDeviceKind.USB -> R.string.install_device_usb_all
        PluginDeviceKind.DISK -> R.string.plugin_device_disk
        PluginDeviceKind.OTHER -> R.string.plugin_device_other
    },
)

private fun grantedDeviceIcon(kind: PluginDeviceKind): ImageVector = when (kind) {
    PluginDeviceKind.SERIAL -> WontonIcon.SerialPort
    PluginDeviceKind.USB -> WontonIcon.Usb
    PluginDeviceKind.DISK -> WontonIcon.Disk
    PluginDeviceKind.OTHER -> WontonIcon.Device
}
