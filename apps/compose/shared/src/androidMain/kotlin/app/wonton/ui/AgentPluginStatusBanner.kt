package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginStartState
import app.wonton.protocol.PluginState
import app.wonton.protocol.RoomPlugin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun AgentPluginStatusBanner(plugin: RoomPlugin?, title: String, onAction: (Action) -> Unit) {
    if (plugin == null) return
    Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)) {
        PluginStatusNotice(
            plugin.pluginId, plugin.status, plugin.start, plugin.canStart, title,
            stringResource(R.string.chat_plugin_stopped), onAction, stringResource(R.string.chat_plugin_starting),
        )
    }
}

@Composable
internal fun PluginDetailStatus(detail: PluginDetail, plugins: PluginsState, working: Boolean) {
    val installed = detail.installed ?: return
    PluginStatusNotice(
        installed.id, installed.status,
        plugins.viewModel.starts.firstOrNull { it.pluginId == installed.id }?.state,
        installed.status is PluginState.Stopped && !working && installed.id !in plugins.viewModel.pending,
        detail.title, stringResource(R.string.tab_stopped, detail.title), plugins::send,
    )
}

@Composable
internal fun PluginStatusNotice(
    pluginId: String,
    status: PluginState,
    start: PluginStartState?,
    canStart: Boolean,
    title: String,
    stoppedText: String,
    onAction: (Action) -> Unit,
    startingText: String = stringResource(R.string.tab_starting),
) {
    val launch = { onAction(Action.StartPlugin(pluginId)) }
    when (start) {
        is PluginStartState.Starting -> PluginStarting(startingText)
        is PluginStartState.Failed -> key(pluginId, start.problem) {
            val problem = start.problem.problem
            WontonNotice(WontonIcon.Warning, problem.titleText, lines = listOfNotNull(problem.nextStepText)) {
                if (canStart) WontonButton(stringResource(R.string.common_retry), launch)
                problem.detail?.let {
                    ProblemDetail(it, modifier = Modifier.heightIn(min = 48.dp))
                }
            }
        }
        null -> PluginStateNotice(status, canStart, title, stoppedText, launch)
    }
}

@Composable
private fun PluginStateNotice(
    status: PluginState,
    canStart: Boolean,
    title: String,
    stoppedText: String,
    launch: () -> Unit,
) {
    when (status) {
        is PluginState.Stopped -> WontonNotice(WontonIcon.PauseFill, stoppedText, isError = false) {
            if (canStart) WontonButton(stringResource(R.string.tab_start), launch)
        }
        is PluginState.Degraded -> WontonNotice(
            WontonIcon.Warning, stringResource(R.string.tab_problem, title),
        ) {
            ProblemDetail(status.reason, modifier = Modifier.heightIn(min = 48.dp))
        }
        is PluginState.NeedsConfiguration -> WontonNotice(
            WontonIcon.Warning, stringResource(R.string.launcher_badge_needs_configuration), isError = false,
        )
        is PluginState.Installing -> PluginStatusLine(stringResource(R.string.install_completion_installing))
        is PluginState.Upgrading -> PluginStatusLine(stringResource(R.string.tab_upgrading))
        is PluginState.Uninstalling -> PluginStatusLine(stringResource(R.string.tab_uninstalling))
        is PluginState.Unknown -> WontonNotice(WontonIcon.Warning, stringResource(R.string.tab_status_unknown))
        is PluginState.Running -> Unit
    }
}

@Composable
private fun PluginStarting(text: String) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = WontonTheme.colors.textPrimary.toColor(),
            strokeWidth = 2.dp,
        )
        PluginStatusLine(text)
    }
}

@Composable
private fun PluginStatusLine(text: String) {
    Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
}
