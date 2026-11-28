package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsViewModel
import app.wonton.protocol.ModelTest
import app.wonton.protocol.ModelTestResult
import app.wonton.protocol.ModelTestState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

private const val MS_PER_SECOND = 1000f

/** 顶上的一块：在用的模型和最近一次测试，下面是“测一下”；没选时只有一句话。 */
@Composable
internal fun CurrentModelCard(view: AgentModelsViewModel, send: (AgentModelsAction) -> Unit) {
    val colors = WontonTheme.colors
    val current = view.current
    val test = view.test
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        WontonGroup {
            Column(
                Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                if (current == null) {
                    Text(
                        stringResource(R.string.agent_models_none),
                        style = WontonTheme.typography.body, color = colors.textSecondary.toColor(),
                    )
                } else {
                    CurrentModelName(view.providerName(current.provider), current.model)
                    test?.let { key(it) { ModelTestLine(it) { send(AgentModelsAction.TestModel) } } }
                }
            }
        }
        if (current != null) {
            WontonButton(
                stringResource(R.string.agent_models_test),
                { send(AgentModelsAction.TestModel) },
                Modifier.fillMaxWidth(),
                kind = WontonButtonKind.SECONDARY,
                enabled = test !is ModelTestState.Running,
            )
            Text(
                stringResource(R.string.agent_models_test_note),
                style = WontonTheme.typography.caption, color = colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
private fun CurrentModelName(provider: String, model: String) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(provider, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        Text(model, style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
    }
}

/** 服务商的名字：列表里有就用它的，没有（被搜索滤掉、目录里已经没有）就原样显示 ID。 */
internal fun AgentModelsViewModel.providerName(id: String): String =
    (recommended + more).firstOrNull { it.id == id }?.name
        ?: provider?.row?.takeIf { it.id == id }?.name
        ?: id

@Composable
private fun ModelTestLine(test: ModelTestState, retry: () -> Unit) {
    val colors = WontonTheme.colors
    when (test) {
        is ModelTestState.Running -> Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(
                Modifier.size(WontonTheme.size.iconSmall),
                color = colors.textPrimary.toColor(), strokeWidth = WontonTheme.size.hairline,
            )
            Text(
                stringResource(R.string.agent_models_test_running),
                style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
            )
        }
        is ModelTestState.Done -> ModelTestResultLine(test.result)
        is ModelTestState.Failed -> ProblemRow(test.problem.problem, retry)
    }
}

@Composable
private fun ModelTestResultLine(test: ModelTest) {
    val colors = WontonTheme.colors
    if (test.result == ModelTestResult.OK) {
        val latency = test.latencyMs?.let {
            String.format(LocalConfiguration.current.locales[0], "%.1f", it.toFloat() / MS_PER_SECOND)
        }
        val text = latency?.let { stringResource(R.string.agent_models_test_ok_latency, it) }
            ?: stringResource(R.string.agent_models_test_ok)
        ResultLine(WontonIcon.Check, colors.success.toColor(), text, null)
    } else {
        ResultLine(WontonIcon.Warning, colors.danger.toColor(), stringResource(test.result.failureText()), test.message)
    }
}

/** 测试没通的一句话，按类别写；引导的确认页用同一套。 */
internal fun ModelTestResult.failureText(): Int = when (this) {
    ModelTestResult.OK -> R.string.agent_models_test_ok
    ModelTestResult.REJECTED -> R.string.agent_models_test_rejected
    ModelTestResult.UNREACHABLE -> R.string.agent_models_test_unreachable
    ModelTestResult.UPSTREAM -> R.string.agent_models_test_upstream
    ModelTestResult.NO_MODEL -> R.string.agent_models_test_no_model
    ModelTestResult.OTHER -> R.string.agent_models_test_other
}

@Composable
private fun ResultLine(icon: ImageVector, tint: Color, text: String, detail: String?) {
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        // 装饰：结果的文字在右边。
        Icon(icon, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textPrimary.toColor())
            detail?.takeIf { it.isNotBlank() }?.let { ProblemDetail(it) }
        }
    }
}
