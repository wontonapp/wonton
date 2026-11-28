package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsState
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding

/** 独立的一页：顶栏加内容。点开一家时整页换成那一家。 */
@Composable
fun AgentModelsScreen(models: AgentModels, onBack: () -> Unit) {
    val page = models.view.provider
    if (page != null) {
        AgentModelProviderScreen(page, models)
        return
    }
    BackHandler(onBack = onBack)
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.agent_models_title), onBack) }) {
        when (val state = models.view.state) {
            is AgentModelsState.Loading -> LoadingView()
            is AgentModelsState.Failed -> ProblemView(state.problem.problem) {
                models.send(AgentModelsAction.OpenAgentModels(models.bot))
            }
            is AgentModelsState.Ready -> Column(
                Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                    .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                AgentModelsList(models.view, models.send)
            }
        }
    }
}
