package app.wonton.ui

import app.wonton.client.PluginInstallAction
import app.wonton.client.deviceChoices
import app.wonton.client.previewInstallAction
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.InstallPreview
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PluginAudienceView
import app.wonton.protocol.PluginDevice
import app.wonton.protocol.WidgetCapability
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme

@Composable
internal fun InstallPermissionChoices(
    lines: List<PermissionLine>,
    selected: Set<String>,
    onChange: (String, Boolean) -> Unit,
    widgetCapabilities: List<WidgetCapability> = emptyList(),
) {
    if (lines.isEmpty() && widgetCapabilities.isEmpty()) return
    DetailSection(stringResource(R.string.install_permissions)) {
        lines.forEach { line ->
            PermissionRow(line) {
                if (line.optional) {
                    WontonSwitch(
                        line.permission in selected,
                        { onChange(line.permission, it) },
                        line.supported,
                        Modifier.semantics { contentDescription = line.text },
                    )
                }
            }
        }
        WidgetCapabilityRows(widgetCapabilities)
    }
}

@Composable
internal fun ConfirmInstallButton(
    preview: InstallPreview,
    title: String,
    secretsReady: Boolean = true,
    onConfirm: () -> Unit,
) {
    var acknowledging by remember(preview.id) { mutableStateOf(false) }
    val rootPermissions = preview.permissions.filter { line ->
        line.needsRootConfirmation && (!line.optional || line.enabled)
    }
    WontonButton(
        title,
        { if (rootPermissions.isEmpty()) onConfirm() else acknowledging = true },
        Modifier.padding(bottom = LocalWontonBottomInset.current)
            .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
        enabled = previewInstallAction(preview) != PluginInstallAction.BLOCKED &&
            preview.missingProfile == null && secretsReady && preview.permissions.all { it.supported } &&
            // 必选的串口没选时不能确认（ADR-0036）。
            deviceChoices(preview.devices.orEmpty(), preview.permissions)?.ready != false,
    )
    if (acknowledging) {
        WontonDialog(
            title = stringResource(R.string.plugin_permission_understand_title),
            onDismiss = { acknowledging = false },
            actions = listOf(
                WontonDialogAction(stringResource(R.string.common_cancel)) { acknowledging = false },
                WontonDialogAction(title, WontonButtonKind.DANGER) {
                    acknowledging = false
                    onConfirm()
                },
            ),
            detail = { rootPermissions.forEach { PermissionRow(it) } },
        )
    }
}

/** 主人在确认页上的选择：局域网开关、打开的可选权限、谁能用、选中的设备。随“安装”一起写回预览交给核心。 */
internal class InstallChoices {
    var lanWebui by mutableStateOf(false)
    var enabled by mutableStateOf<Set<String>>(emptySet())
    var audience by mutableStateOf<PluginAudienceView?>(null)
    var devices by mutableStateOf<List<PluginDevice>>(emptyList())

    /** 换成新取到的预览里的初始值。 */
    fun reset(preview: InstallPreview) {
        lanWebui = preview.lanWebui?.enabled ?: false
        enabled = preview.permissions.filter { it.optional && it.enabled }.map { it.permission }.toSet()
        audience = preview.audience
        devices = preview.devices.orEmpty()
    }

    fun setOptional(permission: String, on: Boolean) {
        enabled = if (on) enabled + permission else enabled - permission
    }

    /** 带上这些选择的预览。预览本来没有设备项（`devices` 为空值）时不补。 */
    fun applyTo(preview: InstallPreview): InstallPreview = preview.copy(
        lanWebui = preview.lanWebui?.copy(enabled = lanWebui),
        permissions = preview.permissions.map { line ->
            if (line.optional) line.copy(enabled = line.permission in enabled) else line
        },
        audience = audience,
        devices = preview.devices?.let { devices },
    )
}
