package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.ModelChoice
import app.wonton.protocol.ModelListState
import app.wonton.protocol.ModelOption
import app.wonton.protocol.ModelProviderPage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 不搜的时候只摆前几个：有的服务商有几百个模型，推荐的那个核心已经排在第一。 */
private const val MODELS_SHOWN = 8

/** 搜出来的最多摆这么多，再多就接着打字。 */
private const val MATCHES_SHOWN = 30

/** 这家的模型：接好了才有。第一个是插件推荐的，预选；点“用这个”才保存。 */
@Composable
internal fun ModelListSection(page: ModelProviderPage, current: ModelChoice?, send: (AgentModelsAction) -> Unit) {
    when (val state = page.models) {
        is ModelListState.NotConnected -> Unit
        is ModelListState.Loading -> ModelsLoading()
        is ModelListState.Failed -> key(state.problem) {
            ProblemRow(state.problem.problem) { send(AgentModelsAction.OpenModelProvider(page.row.id)) }
        }
        is ModelListState.Ready -> DetailSection(stringResource(R.string.agent_models_models)) {
            val inUse = current?.takeIf { it.provider == page.row.id }?.model
            if (state.models.isEmpty()) {
                Text(
                    stringResource(R.string.agent_models_models_empty),
                    style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor(),
                )
            } else {
                key(state.models) { ModelChoices(state.models, inUse, page.busy, send) }
            }
        }
    }
}

@Composable
private fun ModelsLoading() {
    val colors = WontonTheme.colors
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = colors.textPrimary.toColor(), strokeWidth = WontonTheme.size.hairline,
        )
        Text(
            stringResource(R.string.agent_models_models_loading),
            style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun ModelChoices(
    models: List<ModelOption>,
    inUse: String?,
    busy: Boolean,
    send: (AgentModelsAction) -> Unit,
) {
    var selected by remember { mutableStateOf(inUse?.takeIf { id -> models.any { it.id == id } } ?: models.first().id) }
    var filter by remember { mutableStateOf("") }
    val shown = remember(models, filter, selected) { models.shown(filter, selected) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        if (models.size > MODELS_SHOWN) {
            WontonSearchField(filter, { filter = it }, stringResource(R.string.agent_models_models_search, models.size))
        }
        shown.forEach { model ->
            key(model.id) {
                WontonRadioRow(
                    title = model.name?.takeIf { it.isNotBlank() } ?: model.id,
                    selected = model.id == selected,
                    enabled = !busy,
                    onSelect = { selected = model.id },
                    value = if (model.id == inUse) stringResource(R.string.agent_models_in_use) else null,
                )
            }
        }
        if (shown.isEmpty()) {
            Text(
                stringResource(R.string.agent_models_no_match),
                style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
        WontonButton(
            stringResource(R.string.agent_models_use),
            { send(AgentModelsAction.SelectModel(selected)) },
            Modifier.fillMaxWidth(),
            enabled = !busy && selected != inUse,
        )
    }
}

/** 摆出来的那几个：不搜时是前几个加选中的那个，搜的时候是名字或 ID 含搜索词的。 */
private fun List<ModelOption>.shown(filter: String, selected: String): List<ModelOption> {
    val text = filter.trim()
    if (text.isEmpty()) {
        val head = take(MODELS_SHOWN)
        return if (head.any { it.id == selected }) head else head + filter { it.id == selected }
    }
    return asSequence()
        .filter { it.id.contains(text, ignoreCase = true) || it.name?.contains(text, ignoreCase = true) == true }
        .take(MATCHES_SHOWN)
        .toList()
}
