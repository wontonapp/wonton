package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAgentRow
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingViewModel
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun OnboardingScreen(
    view: OnboardingViewModel,
    plugins: PluginsState,
    onConfirm: (PluginConfirmation, () -> Unit) -> Unit,
    openGateway: (String) -> Unit,
) {
    val nav = requireNotNull(view.nav)
    val send = plugins::send
    BackHandler { if (nav.canBack) send(Action.BackOnboardingStep) }
    val modifier = Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())
        .statusBarsPadding().navigationBarsPadding().imePadding()
    if (view.step == OnboardingStep.PROFILE) {
        OnboardingProfileStep(view, nav, plugins.core, send, modifier)
        return
    }
    val scroll = rememberScrollState()
    LaunchedEffect(view.step) { scroll.scrollTo(0) }
    LaunchedEffect(view.problem, view.model.problem) {
        if (view.problem != null || view.model.problem != null) scroll.scrollTo(0)
    }
    StepCardFrame(
        nav = nav,
        title = view.title,
        primaryText = if (view.step == OnboardingStep.APPS) "完成" else "下一步",
        primaryBusy = view.loading || view.model.saving != null,
        onBack = { send(Action.BackOnboardingStep) },
        onSkip = { send(Action.SkipOnboardingStep) },
        onPrimary = {
            val source = view.agents.firstOrNull { it.selected }?.source
            if (view.step == OnboardingStep.AGENT && source != null) {
                onConfirm(PluginConfirmation.Install(source)) { send(Action.ContinueOnboarding) }
            } else {
                send(Action.ContinueOnboarding)
            }
        },
        modifier = modifier,
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(scroll),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                val problem = view.problem
                if (problem != null) {
                    OnboardingProblem(problem.onboardingTitle(), problem.message) { send(Action.RefreshOnboarding) }
                } else {
                    when (view.step) {
                        OnboardingStep.AGENT -> OnboardingAgentStep(view.agents, send)
                        OnboardingStep.MODEL -> OnboardingModelStep(view.model, send, openGateway)
                        OnboardingStep.APPS -> OnboardingAppsStep(view, plugins, send) { onConfirm(it) {} }
                        else -> Unit
                    }
                }
            }
        }
    }
}

@Composable
private fun OnboardingAgentStep(agents: List<OnboardingAgentRow>, send: (Action) -> Unit) {
    if (agents.isEmpty()) {
        OnboardingProblem("没有可选的 Agent，可以刷新重试。", null) { send(Action.RefreshOnboarding) }
    }
    agents.forEach { agent ->
        key(agent.id) {
            StepOptionRow(
                title = agent.title,
                description = agent.connection,
                selected = agent.selected,
                blocked = agent.blocked,
                onSelect = { send(Action.ChooseOnboardingAgent(agent.id)) },
            )
        }
    }
}
