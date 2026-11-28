package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsState
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StepNav
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTheme

/**
 * 接模型这一步就是默认 Bot 的模型页（ADR-0129 第 4 节）：操作当场存进 Bot，不进引导草稿；能不能点下一步看核心给的
 * [StepNav]。点开一家服务商时整页换成那一家，返回回到这一步。
 */
@Composable
internal fun OnboardingModelStep(
    view: OnboardingViewModel,
    nav: StepNav,
    plugins: PluginsState,
    onClose: () -> Unit,
    modifier: Modifier,
) {
    val bot = view.model.bot
    if (bot == null) {
        OnboardingModelStepCard(view, nav, null, plugins::send, onClose, modifier)
        return
    }
    AgentModelsHost(plugins.core, bot) { models ->
        OnboardingModelStepCard(view, nav, models, plugins::send, onClose, modifier)
    }
}

/** 这一步画出来的样子；[models] 为空是核心没给默认 Bot。走查页用假数据直接画它。 */
@Composable
fun OnboardingModelStepCard(
    view: OnboardingViewModel,
    nav: StepNav,
    models: AgentModels?,
    send: (Action) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val page = models?.view?.provider
    if (models != null && page != null) {
        AgentModelProviderScreen(page, models)
        return
    }
    StepCardFrame(
        nav = nav,
        title = view.title(),
        primaryText = stringResource(R.string.onboarding_next),
        primaryBusy = view.loading,
        onBack = { send(Action.Onboarding(OnboardingAction.BackOnboardingStep)) },
        onSkip = { send(Action.Onboarding(OnboardingAction.SkipOnboardingStep)) },
        onPrimary = { send(Action.Onboarding(OnboardingAction.ContinueOnboarding)) },
        onClose = onClose,
        modifier = modifier,
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                val problem = view.problem
                if (problem != null) {
                    OnboardingProblem(problem.onboardingTitle(), problem.message) {
                        send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                    }
                } else if (models != null) {
                    ModelStepContent(models)
                }
            }
        }
    }
}

@Composable
private fun ModelStepContent(models: AgentModels) {
    when (val state = models.view.state) {
        is AgentModelsState.Loading -> OnboardingLoading()
        is AgentModelsState.Failed -> ProblemView(state.problem.problem) {
            models.send(AgentModelsAction.OpenAgentModels(models.bot))
        }
        is AgentModelsState.Ready -> AgentModelsList(models.view, models.send)
    }
}
