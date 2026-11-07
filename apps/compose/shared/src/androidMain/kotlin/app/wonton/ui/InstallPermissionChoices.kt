package app.wonton.ui

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
) {
    if (lines.isEmpty()) return
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
    }
}

@Composable
internal fun ConfirmInstallButton(preview: InstallPreview, title: String, onConfirm: () -> Unit) {
    var acknowledging by remember(preview.id) { mutableStateOf(false) }
    val rootPermissions = preview.permissions.filter { line ->
        line.needsRootConfirmation && (!line.optional || line.enabled)
    }
    WontonButton(
        title,
        { if (rootPermissions.isEmpty()) onConfirm() else acknowledging = true },
        Modifier.padding(bottom = LocalWontonBottomInset.current)
            .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
        enabled = preview.blocked == null && preview.missingProfile == null &&
            preview.permissions.all { it.supported },
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

internal fun InstallPreview.withPermissionChoices(lan: Boolean, selected: Set<String>): InstallPreview = copy(
    lanWebui = lanWebui?.copy(enabled = lan),
    permissions = permissions.map { line ->
        if (line.optional) line.copy(enabled = line.permission in selected) else line
    },
)
