package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import app.wonton.client.DeviceChoices
import app.wonton.protocol.PluginDevice
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 确认页和确认卡片上的“设备”一节（ADR-0036）：串口每个服务选一个，USB 整类授权，硬盘勾选、系统盘有标记。
 * 怎么分组、点一项以后谁选中由核心定（`deviceChoices`、`toggleDevice`），这里只画、不调核心；原始的设备 ID 和
 * 容器里的路径收在“详情”里。
 */
@Composable
fun DeviceChoicesSection(choices: DeviceChoices, enabled: Boolean = true, onToggle: (String) -> Unit) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.install_devices)) {
        choices.serial.forEach { group ->
            DeviceNote(stringResource(R.string.install_device_serial_for, group.service))
            group.devices.forEach { DeviceRow(it, multiple = false, enabled, onToggle) }
        }
        if (choices.serialMissing) DeviceNote(stringResource(R.string.install_device_serial_none))
        if (choices.usb) {
            Text(
                stringResource(R.string.install_device_usb_all),
                Modifier.testTag("device-usb-all"),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
        }
        choices.disks.forEach { DeviceRow(it, multiple = true, enabled, onToggle) }
        if (choices.disksMissing) DeviceNote(stringResource(R.string.install_device_disks_none))
        choices.unknown.forEach { device ->
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(device.name, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                DeviceNote(stringResource(R.string.install_device_update_app))
            }
        }
        rawDeviceValues(choices).takeIf { it.isNotEmpty() }?.let { ProblemDetail(it) }
    }
}

@Composable
private fun DeviceRow(device: PluginDevice, multiple: Boolean, enabled: Boolean, onToggle: (String) -> Unit) {
    StepOptionRow(
        device.name,
        selected = device.chosen,
        onSelect = if (enabled) ({ onToggle(device.id) }) else null,
        modifier = Modifier.testTag("device-${device.id}"),
        multiple = multiple,
        trailing = {
            if (device.system) Tag(stringResource(R.string.install_device_system_disk), WontonTheme.colors.danger)
        },
    )
}

@Composable
private fun DeviceNote(text: String) {
    Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
}

/** 排查时才看的原始值：核心认设备的 ID、交给哪个服务、在容器里的路径。 */
private fun rawDeviceValues(choices: DeviceChoices): String =
    (choices.serial.flatMap { it.devices } + choices.disks + choices.unknown).joinToString("\n\n") { device ->
        listOf(
            device.name,
            device.id,
            listOf(device.service, device.target).filter { it.isNotEmpty() }.joinToString(" "),
        ).joinToString("\n")
    }
