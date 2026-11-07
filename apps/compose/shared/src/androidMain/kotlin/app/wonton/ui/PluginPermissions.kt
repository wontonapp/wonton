package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.material3.Text
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PermissionRisk
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Shield
import app.wonton.ui.icons.WarningFill
import app.wonton.ui.icons.WontonIcon

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun PermissionRow(line: PermissionLine, trailing: @Composable () -> Unit = {}) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LineIcon(
                if (line.risk == null) WontonIcon.Shield else WontonIcon.WarningFill,
                (if (line.risk == null) colors.textSecondary else colors.warning).toColor(),
            )
            Text(
                line.text,
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
            trailing()
        }
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            line.risk?.let { Tag(permissionRiskText(it), colors.warning) }
            if (line.optional) Tag(stringResource(R.string.plugin_permission_optional), colors.textSecondary)
            if (line.added) Tag(stringResource(R.string.plugin_permission_added), colors.primary)
            if (!line.supported) Tag(stringResource(R.string.plugin_permission_unsupported), colors.danger)
        }
        line.notice?.let { PermissionSupportingText(it) }
        line.offLoss?.let { PermissionSupportingText(it) }
    }
}

@Composable
internal fun PermissionSupportingText(text: String) {
    Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
}

@Composable
private fun permissionRiskText(risk: PermissionRisk): String = stringResource(
    when (risk) {
        PermissionRisk.HIGH -> R.string.plugin_permission_risk_high
        PermissionRisk.VERY_HIGH -> R.string.plugin_permission_risk_very_high
        PermissionRisk.HIGHEST, PermissionRisk.OTHER -> R.string.plugin_permission_risk_highest
    },
)

internal val PermissionLine.needsRootConfirmation: Boolean
    get() = risk == PermissionRisk.HIGHEST || risk == PermissionRisk.OTHER

@Composable
internal fun PermissionChangeConfirmation(
    line: PermissionLine,
    enabling: Boolean,
    onDismiss: () -> Unit,
    onConfirmed: () -> Unit,
) {
    var secondStep by remember(line.permission, enabling) { mutableStateOf(false) }
    val needsSecond = enabling && line.needsRootConfirmation
    val title = when {
        secondStep -> stringResource(R.string.plugin_permission_understand_title)
        enabling -> stringResource(R.string.plugin_permission_open_title, line.text)
        else -> stringResource(R.string.plugin_permission_close_title, line.text)
    }
    val confirm = when {
        needsSecond && !secondStep -> R.string.plugin_permission_continue
        enabling -> R.string.plugin_permission_open
        else -> R.string.plugin_permission_close
    }
    WontonDialog(
        title = title,
        message = if (enabling) line.notice else line.offLoss,
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(
                stringResource(confirm),
                if (needsSecond || !enabling) WontonButtonKind.DANGER else WontonButtonKind.PRIMARY,
            ) {
                if (needsSecond && !secondStep) secondStep = true else onConfirmed()
            },
        ),
    )
}
