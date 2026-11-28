package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.CapabilityProviderRef
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginAction
import app.wonton.protocol.UninstallOutcome
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 卸载前确认（ADR-0053）：先问核心能不能卸。可以卸时列出没人再用的提供方；被在用的依赖挡住时列出依赖方。 */
@Composable
internal fun UninstallDialog(target: UninstallTarget, plugins: PluginsState, onDismiss: () -> Unit) {
    var outcome by remember(target) { mutableStateOf<UninstallOutcome?>(null) }
    var problem by remember(target) { mutableStateOf<Problem?>(null) }
    var chosen by remember(target) { mutableStateOf<Set<String>>(emptySet()) }
    var attempt by remember(target) { mutableIntStateOf(0) }
    LaunchedEffect(target, attempt) {
        outcome = null
        problem = null
        chosen = emptySet()
        try {
            outcome = plugins.core.uninstallPreview(target.id)
        } catch (failure: CoreException) {
            problem = failure.problem
        }
    }
    val title = if (target.keepDataOnly) {
        stringResource(R.string.plugin_clear_failure_title, target.title)
    } else {
        stringResource(R.string.plugin_uninstall_title, target.title)
    }
    val uninstall = { deleteData: Boolean, dependents: List<String> ->
        onDismiss()
        plugins.send(Action.Plugins(PluginAction.UninstallPlugin(
            id = target.id,
            deleteData = deleteData,
            providers = chosen.toList(),
            dependents = dependents,
        )))
    }
    when {
        problem != null -> UninstallFailedDialog(title, problem, onDismiss) { attempt++ }
        outcome == null -> UninstallCheckingDialog(title, onDismiss)
        outcome is UninstallOutcome.InUse -> UninstallInUseDialog(
            title, target, (outcome as UninstallOutcome.InUse).dependents, onDismiss,
            onTogether = { dependents -> uninstall(false, dependents) },
            onStop = { dependents ->
                onDismiss()
                dependents.forEach { plugins.send(Action.Plugins(PluginAction.StopPlugin(it))) }
            },
        )
        else -> UninstallReadyDialog(
            title, target, (outcome as UninstallOutcome.Ready).unusedProviders, chosen,
            onToggle = { id -> chosen = if (id in chosen) chosen - id else chosen + id },
            onDismiss = onDismiss,
            onUninstall = uninstall,
        )
    }
}

@Composable
private fun UninstallFailedDialog(title: String, problem: Problem?, onDismiss: () -> Unit, retry: () -> Unit) {
    WontonDialog(
        title,
        stringResource(R.string.plugin_uninstall_preview_failed),
        onDismiss,
        listOf(
            WontonDialogAction(stringResource(R.string.common_retry)) { retry() },
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
        ),
        detail = { problem?.let { ProblemNotice(it) } },
    )
}

@Composable
private fun UninstallCheckingDialog(title: String, onDismiss: () -> Unit) {
    WontonDialog(title, onDismiss = onDismiss, detail = {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon))
            Text(stringResource(R.string.plugin_uninstall_checking))
        }
    })
}

@Composable
private fun UninstallInUseDialog(
    title: String,
    target: UninstallTarget,
    dependents: List<CapabilityProviderRef>,
    onDismiss: () -> Unit,
    onTogether: (List<String>) -> Unit,
    onStop: (List<String>) -> Unit,
) {
    WontonDialog(
        title,
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.plugin_uninstall_together)) {
                onTogether(dependents.map { it.id })
            },
            WontonDialogAction(stringResource(R.string.plugin_uninstall_stop_dependents)) {
                onStop(dependents.map { it.id })
            },
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
        ),
        detail = {
            WontonNotice(
                WontonIcon.Warning,
                stringResource(R.string.plugin_uninstall_in_use, target.title),
                isError = false,
                lines = dependents.map { it.title },
            )
            Text(
                stringResource(R.string.plugin_uninstall_together_keeps_data),
                style = WontonTheme.typography.supporting,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        },
    )
}

@Composable
private fun UninstallReadyDialog(
    title: String,
    target: UninstallTarget,
    unused: List<CapabilityProviderRef>,
    chosen: Set<String>,
    onToggle: (String) -> Unit,
    onDismiss: () -> Unit,
    onUninstall: (Boolean, List<String>) -> Unit,
) {
    WontonDialog(
        title,
        if (target.keepDataOnly) stringResource(R.string.plugin_clear_failure_message)
        else stringResource(R.string.plugin_uninstall_delete_data_warning),
        onDismiss,
        listOfNotNull(
            WontonDialogAction(
                stringResource(
                    if (target.keepDataOnly) R.string.plugin_clear_failure
                    else R.string.plugin_uninstall_keep_data,
                ),
            ) { onUninstall(false, emptyList()) },
            if (target.keepDataOnly) null else WontonDialogAction(
                stringResource(R.string.plugin_uninstall_delete_data),
                WontonButtonKind.DANGER,
            ) { onUninstall(true, emptyList()) },
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
        ),
        detail = if (unused.isEmpty()) null else {
            {
                Text(
                    stringResource(R.string.plugin_uninstall_unused_providers),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
                unused.forEach { provider ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            provider.title,
                            Modifier.weight(1f),
                            style = WontonTheme.typography.body,
                            color = WontonTheme.colors.textPrimary.toColor(),
                        )
                        WontonSwitch(provider.id in chosen, { on -> onToggle(provider.id) }, true)
                    }
                }
            }
        },
    )
}

