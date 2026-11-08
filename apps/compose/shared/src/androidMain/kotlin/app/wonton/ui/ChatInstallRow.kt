package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.BotInstallRow
import app.wonton.protocol.BotInstallState
import app.wonton.protocol.InstallStage
import app.wonton.protocol.PluginOperation
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.NumberFormat

private val SPINNER_SIZE = 16.dp
private val SPINNER_STROKE = 2.dp
private const val PERCENT_FULL = 100f

/**
 * 聊天列表最上面的安装行（ADR-0067）：正在装、正在升级、装失败。只渲染核心给的行，点重试交给确认页，
 * 来源不知道时引导去应用页。
 */
@Composable
internal fun ChatInstallRow(
    row: BotInstallRow,
    plugins: PluginsState,
    onRetry: () -> Unit,
    onOpenPage: () -> Unit,
) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        PluginIconImage(row.icon, WontonTheme.size.avatar, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Text(
                    row.title,
                    Modifier.weight(1f),
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val progress = (row.state as? BotInstallState.Running)?.progress
                if (progress != null) {
                    val percent = progress.percent
                    if (percent == null) {
                        CircularProgressIndicator(
                            Modifier.size(SPINNER_SIZE),
                            color = colors.textSecondary.toColor(),
                            strokeWidth = SPINNER_STROKE,
                        )
                    } else {
                        Text(
                            NumberFormat.getPercentInstance(LocalConfiguration.current.locales[0])
                                .format(percent.toDouble() / PERCENT_FULL),
                            style = WontonTheme.typography.caption.copy(fontFeatureSettings = "tnum"),
                            color = colors.textSecondary.toColor(),
                        )
                    }
                }
            }
            when (val state = row.state) {
                is BotInstallState.Running -> RunningLine(row.operation, state)
                is BotInstallState.Failed -> FailedLine(row, state, onRetry, onOpenPage)
                is BotInstallState.Uncertain -> UncertainLine(state.message, onOpenPage)
            }
        }
    }
}

@Composable
private fun RunningLine(operation: PluginOperation, state: BotInstallState.Running) {
    val progress = state.progress
    Text(
        stageText(progress.stage, operation),
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
    progress.percent?.let {
        LinearProgressIndicator(progress = { it.toFloat() / PERCENT_FULL }, Modifier.fillMaxWidth())
    }
}

@Composable
private fun FailedLine(
    row: BotInstallRow,
    state: BotInstallState.Failed,
    onRetry: () -> Unit,
    onOpenPage: () -> Unit,
) {
    val colors = WontonTheme.colors
    val problem = state.reason.problem(state.message)
    Text(
        stringResource(
            when (row.operation) {
                PluginOperation.UPGRADE -> R.string.plugin_task_upgrade_failed
                else -> R.string.plugin_task_install_failed
            },
            problem.titleText,
        ),
        style = WontonTheme.typography.supporting,
        color = colors.danger.toColor(),
    )
    problem.nextStepText?.let {
        Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
    }
    ProblemDetail(state.message)
    Row {
        // 升级不重新预览来源，装的时候来源不知道就不能重试。
        if (row.operation == PluginOperation.UPGRADE || row.source != null) {
            WontonSmallButton(stringResource(R.string.common_retry), onRetry)
        } else {
            WontonSmallButton(stringResource(R.string.chat_install_open_page), onOpenPage)
        }
    }
}

@Composable
private fun UncertainLine(message: String, onOpenPage: () -> Unit) {
    Text(
        stringResource(R.string.plugin_task_uncertain, message),
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.warning.toColor(),
    )
    Row { WontonSmallButton(stringResource(R.string.chat_install_open_page), onOpenPage) }
}

/** 阶段由核心给，不认识的阶段落到这个操作通用的进行时。 */
@Composable
private fun stageText(stage: InstallStage, operation: PluginOperation): String = when (stage) {
    InstallStage.PULLING_IMAGES -> stringResource(R.string.chat_install_stage_pulling_images)
    InstallStage.PREPARING -> stringResource(R.string.chat_install_stage_preparing)
    InstallStage.STARTING -> stringResource(R.string.chat_install_stage_starting)
    InstallStage.WAITING_FOR_BOT -> stringResource(R.string.chat_install_stage_waiting_for_bot)
    InstallStage.OTHER -> operation.runningText
}
