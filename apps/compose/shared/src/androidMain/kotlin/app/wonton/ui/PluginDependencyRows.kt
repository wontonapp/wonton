package app.wonton.ui

import androidx.compose.runtime.Composable
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState

/** 依赖异常、能力提供方选择和卸载被挡（ADR-0053）。 */
@Composable
internal fun DependencySections(
    detail: PluginDetail,
    plugins: PluginsState,
    task: PluginTask?,
    isInstalled: Boolean,
    isOwner: Boolean,
    onUpdate: (PluginDetail) -> Unit,
) {
    PluginDependencyHints(detail, plugins, onUpdate)
    if (isInstalled && isOwner && detail.capabilityProviders.isNotEmpty()) {
        PluginCapabilityChoices(detail, plugins, onUpdate)
    }
    val dependents = (task?.state as? PluginTaskState.Failed)
        ?.takeIf { it.reason == PluginFailure.IN_USE }
        ?.let { task?.dependents.orEmpty() }
        .orEmpty()
    if (dependents.isNotEmpty()) {
        UninstallDependents(plugins, detail.title, detail.id, dependents)
    }
}

