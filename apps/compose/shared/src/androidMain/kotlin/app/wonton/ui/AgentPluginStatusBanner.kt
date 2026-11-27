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
import app.wonton.protocol.PluginAction
import app.wonton.protocol.Action
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.PluginStartState
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginsViewModel
import app.wonton.protocol.RoomPlugin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun AgentPluginStatusBanner(
    plugin: RoomPlugin?,
    title: String,
    plugins: PluginsViewModel,
    onAction: (Action) -> Unit,
) {
    if (plugin == null) return
    // 安装失败并回滚的插件带着降级状态，但不是“已安装”；列表没到时无从判断，照旧显示。
    val installFailed = plugins.installed?.plugins
        ?.firstOrNull { it.id == plugin.pluginId }?.installFailure != null
    Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)) {
        PluginStatusNotice(
            plugin.pluginId, plugin.status, plugin.start, plugin.canStart, title,
            stringResource(R.string.chat_plugin_stopped), onAction, stringResource(R.string.chat_plugin_starting),
            degradedLine = !installFailed,
        )
    }
}

@Composable
internal fun PluginDetailStatus(detail: PluginDetail, plugins: PluginsState, working: Boolean) {
    val installed = detail.installed ?: return
    val failure = installed.installFailure
    if (failure != null) {
        val task = plugins.viewModel.tasks.firstOrNull { it.id == installed.id }
        val reason = (task?.state as? PluginTaskState.Failed)?.reason
            ?: plugins.core.pluginInstallFailureReason(failure.kind)
        val problem = reason.problem(failure.message)
        ProblemNotice(Problem(
            title = stringResource(R.string.plugin_task_install_failed, problem.titleText),
            nextStep = problem.nextStepText,
            detail = problem.detail,
        ))
        val settled = task?.state !is PluginTaskState.Uncertain &&
            installed.id !in plugins.viewModel.pending
        if (failure.retryable && !working && settled) {
            WontonButton(stringResource(R.string.common_retry), onClick = {
                plugins.send(Action.Plugins(PluginAction.RetryPluginInstall(id = installed.id)))
            })
        }
    } else {
        PluginStatusNotice(
            installed.id, installed.status,
            plugins.viewModel.starts.firstOrNull { it.pluginId == installed.id }?.state,
            installed.status is PluginState.Stopped && !working && installed.id !in plugins.viewModel.pending,
            detail.title, stringResource(R.string.tab_stopped, detail.title), plugins::send,
        )
    }
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
    degradedLine: Boolean = true,
) {
    val launch = { onAction(Action.Plugins(PluginAction.StartPlugin(pluginId))) }
    when (start) {
        is PluginStartState.Starting -> PluginStarting(startingText)
        is PluginStartState.Failed -> key(pluginId, start.problem) {
            val problem = start.problem.problem
            WontonNotice(WontonIcon.Warning, problem.titleText, lines = listOfNotNull(problem.nextStepText)) {
                if (canStart) WontonButton(stringResource(R.string.common_retry), launch)
                problem.detail?.let { DetailInCard(it) }
            }
        }
        null -> PluginStateNotice(status, canStart, title, stoppedText, launch, degradedLine)
    }
}

@Composable
private fun PluginStateNotice(
    status: PluginState,
    canStart: Boolean,
    title: String,
    stoppedText: String,
    launch: () -> Unit,
    degradedLine: Boolean,
) {
    when (status) {
        is PluginState.Stopped -> WontonNotice(WontonIcon.PauseFill, stoppedText, isError = false) {
            if (canStart) WontonButton(stringResource(R.string.tab_start), launch)
        }
        is PluginState.Degraded -> WontonNotice(
            WontonIcon.Warning, stringResource(R.string.tab_problem, title),
            lines = if (degradedLine) listOf(stringResource(R.string.tab_degraded_line)) else emptyList(),
        ) {
            // 原因是核心按语言写的，个别场景夹着原始报错；没有内容时不给一个展开是空的详情。
            if (status.reason.isNotBlank()) {
                DetailInCard(status.reason)
            }
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

/**
 * 卡片里收起的详情：内容再长也只长到 [DETAIL_MAX_HEIGHT] 就在里面滚。卡片钉在聊天顶上，
 * 不封顶会把同屏的照片和文字挤出列表（#701）。
 */
@Composable
private fun DetailInCard(text: String) {
    ProblemDetail(
        text,
        modifier = Modifier.heightIn(min = 48.dp),
        toggleTag = "agent-detail-toggle",
        expandedMaxHeight = DETAIL_MAX_HEIGHT,
    )
}

/** 够看四行左右；再长的原因在详情里滚，卡片至多长高这么多。 */
private val DETAIL_MAX_HEIGHT = 72.dp

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
