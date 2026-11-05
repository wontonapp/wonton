package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier

import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.StoreItem
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun OnboardingAppsStep(
    view: OnboardingViewModel,
    plugins: PluginsState,
    send: (Action) -> Unit,
    onConfirm: (PluginConfirmation) -> Unit,
) {
    view.catalogProblem?.let {
        OnboardingProblem(it.onboardingTitle(), it.details) { send(Action.RefreshOnboarding) }
    }
    view.needs.forEach { need ->
        key(need.id) {
            StepOptionRow(
                title = need.title,
                selected = need.selected,
                onSelect = { send(Action.ToggleOnboardingNeed(need.id)) },
                multiple = true,
            )
            if (need.selected) {
                Column(
                    Modifier.padding(start = WontonTheme.spacing.s16),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    need.plugins.forEach { item ->
                        key(item.id) { OnboardingAppRow(item, plugins, onConfirm) }
                    }
                }
            }
        }
    }
    // Failed work stays visible even when its need is no longer selected.
    plugins.viewModel.tasks.filter { it.state is PluginTaskState.Failed || it.state is PluginTaskState.Uncertain }
        .forEach { task ->
            key(task.id) {
                TaskRow(task, plugins) {
                    val confirmation = view.needs.asSequence().flatMap { it.plugins.asSequence() }
                        .firstOrNull { it.id == task.id }?.let { PluginConfirmation.Install(it.source) }
                        ?: reconfirmation(task, plugins)
                    confirmation?.let(onConfirm)
                }
            }
        }
}

@Composable
private fun OnboardingAppRow(item: StoreItem, plugins: PluginsState, onConfirm: (PluginConfirmation) -> Unit) {
    StepOptionRow(
        title = item.title,
        selected = false,
        onSelect = null,
        blocked = item.blocked,
        leading = { PluginIconImage(item.icon, WontonTheme.size.taskIcon, plugins) },
        trailing = {
            val busy = plugins.viewModel.hasRunningTask(item.id)
            when {
                busy || item.installedVersion != null -> Text(
                    if (busy) "安装中" else "已安装",
                    Modifier.padding(WontonTheme.spacing.s12),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
                else -> WontonButton(
                    "安装",
                    { onConfirm(PluginConfirmation.Install(item.source)) },
                    Modifier.padding(end = WontonTheme.spacing.s8),
                    kind = WontonButtonKind.SECONDARY,
                    enabled = item.blocked == null,
                )
            }
        },
    )
}
