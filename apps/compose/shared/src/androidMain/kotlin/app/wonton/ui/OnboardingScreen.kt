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
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAgentRow
import app.wonton.protocol.OnboardingCommit
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
    openGateway: (String) -> Unit,
) {
    val nav = requireNotNull(view.nav)
    val send = plugins::send
    BackHandler { if (nav.canBack) send(Action.BackOnboardingStep) }
    val modifier = Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())
        .statusBarsPadding().navigationBarsPadding().imePadding()
    if (view.step == OnboardingStep.PROFILE) {
        OnboardingProfileStep(view, nav, send, modifier)
        return
    }
    val scroll = rememberScrollState()
    val modelForm = remember(view.step) { OnboardingModelForm(view.model.choice) }
    LaunchedEffect(view.step) { scroll.scrollTo(0) }
    LaunchedEffect(view.problem, view.model.problem) {
        if (view.problem != null || view.model.problem != null) scroll.scrollTo(0)
    }
    StepCardFrame(
        nav = if (view.step == OnboardingStep.MODEL) {
            nav.copy(canNext = nav.canNext || (!view.model.testing && modelForm.canTest))
        } else nav,
        title = view.title,
        primaryText = view.primaryText(),
        primaryBusy = view.loading || view.model.testing || view.review.commit == OnboardingCommit.RUNNING,
        onBack = { send(Action.BackOnboardingStep) },
        onSkip = { send(Action.SkipOnboardingStep) },
        onPrimary = {
            if (view.step == OnboardingStep.MODEL) {
                modelForm.submit(send, thenContinue = true)
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
                        OnboardingStep.MODEL -> OnboardingModelStep(view.model, modelForm, send, openGateway)
                        OnboardingStep.APPS -> OnboardingAppsStep(view, plugins, send)
                        OnboardingStep.REVIEW -> OnboardingReviewStep(view.review, plugins, send)
                        else -> Unit
                    }
                }
            }
        }
    }
}

/** 每一步卡片顶上的标题。 */
internal val OnboardingViewModel.title: String
    get() = when (step) {
        null -> ""
        OnboardingStep.PROFILE -> "你的资料"
        OnboardingStep.AGENT -> "选 Agent"
        OnboardingStep.MODEL -> "接模型"
        OnboardingStep.APPS -> "选应用"
        OnboardingStep.REVIEW -> "确认"
        OnboardingStep.FINISHED -> "完成"
        OnboardingStep.OTHER -> "引导"
    }

/** 接模型的方式，如“ChatGPT 账号或 key”；没有账号登录时只有 key。 */
internal fun OnboardingAgentRow.connection(): String =
    if (logins.isEmpty()) "key" else logins.joinToString("、") + "或 key"

private fun OnboardingViewModel.primaryText(): String = if (step == OnboardingStep.REVIEW) {
    when (review.commit) {
        OnboardingCommit.NOT_STARTED -> "完成"
        OnboardingCommit.RUNNING -> "进行中"
        OnboardingCommit.DONE -> "进入聊天"
    }
} else {
    "下一步"
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
                description = agent.connection(),
                selected = agent.selected,
                blocked = agent.blocked,
                onSelect = { send(Action.ChooseOnboardingAgent(agent.id)) },
            )
        }
    }
}
