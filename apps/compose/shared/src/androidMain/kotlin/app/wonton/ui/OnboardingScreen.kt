package app.wonton.ui

import android.icu.text.ListFormatter
import android.os.Build
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
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
    val onboarding = LocalOnboarding.current
    // 系统返回和左边缘右滑同一路：能后退就后退，重看引导的第一步等同关闭。
    BackHandler {
        when {
            nav.canBack -> send(Action.Onboarding(OnboardingAction.BackOnboardingStep))
            nav.canClose -> onboarding.close()
        }
    }
    val modifier = Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())
        .statusBarsPadding().navigationBarsPadding().imePadding()
    if (view.step == OnboardingStep.PROFILE) {
        OnboardingProfileStep(view, nav, send, modifier)
        return
    }
    val scroll = rememberScrollState()
    LaunchedEffect(view.step) { scroll.scrollTo(0) }
    LaunchedEffect(view.problem, view.model.problem) {
        if (view.problem != null || view.model.problem != null) scroll.scrollTo(0)
    }
    StepCardFrame(
        nav = nav,
        title = view.title(),
        primaryText = view.primaryText(),
        primaryBusy = view.loading || view.review.commit == OnboardingCommit.RUNNING,
        onBack = { send(Action.Onboarding(OnboardingAction.BackOnboardingStep)) },
        onSkip = { send(Action.Onboarding(OnboardingAction.SkipOnboardingStep)) },
        onPrimary = { send(Action.Onboarding(OnboardingAction.ContinueOnboarding)) },
        onClose = onboarding.close,
        modifier = modifier,
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(scroll),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                val problem = view.problem
                if (problem != null) {
                    OnboardingProblem(problem.onboardingTitle(), problem.message) {
                        send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                    }
                } else {
                    when (view.step) {
                        OnboardingStep.AGENT -> OnboardingAgentStep(view.agents, send)
                        OnboardingStep.MODEL -> OnboardingModelStep(view.model, send, openGateway)
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
@Composable
internal fun OnboardingViewModel.title(): String = when (step) {
    null -> ""
    OnboardingStep.PROFILE -> stringResource(R.string.onboarding_step_profile)
    OnboardingStep.AGENT -> stringResource(R.string.onboarding_step_agent)
    OnboardingStep.MODEL -> stringResource(R.string.onboarding_step_model)
    OnboardingStep.APPS -> stringResource(R.string.onboarding_step_apps)
    OnboardingStep.REVIEW -> stringResource(R.string.onboarding_step_review)
    OnboardingStep.FINISHED -> stringResource(R.string.onboarding_step_finished)
    OnboardingStep.OTHER -> stringResource(R.string.onboarding_step_other)
}

/** 接模型的方式，如“ChatGPT 账号或 key”；没有账号登录时只有 key。 */
@Composable
internal fun OnboardingAgentRow.connection(): String {
    if (logins.isEmpty()) return stringResource(R.string.onboarding_agent_connection_key)
    val locale = LocalConfiguration.current.locales[0]
    // ListFormatter 的“或”型从 API 33 才有；更早的系统退回默认（“和”）型，只有登录名多到两个以上才有差别。
    val formatted = if (Build.VERSION.SDK_INT >= 33) {
        ListFormatter.getInstance(locale, ListFormatter.Type.OR, ListFormatter.Width.WIDE).format(logins)
    } else {
        ListFormatter.getInstance(locale).format(logins)
    }
    return stringResource(R.string.onboarding_agent_connection_logins, formatted)
}

@Composable
private fun OnboardingViewModel.primaryText(): String = if (step == OnboardingStep.REVIEW) {
    when (review.commit) {
        OnboardingCommit.NOT_STARTED -> stringResource(R.string.onboarding_done)
        OnboardingCommit.RUNNING -> stringResource(R.string.onboarding_running)
        OnboardingCommit.DONE -> stringResource(R.string.onboarding_enter_chat)
    }
} else {
    stringResource(R.string.onboarding_next)
}

@Composable
private fun OnboardingAgentStep(agents: List<OnboardingAgentRow>, send: (Action) -> Unit) {
    if (agents.isEmpty()) {
        OnboardingProblem(stringResource(R.string.onboarding_agent_empty), null) {
            send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
        }
    }
    agents.forEach { agent ->
        key(agent.id) {
            StepOptionRow(
                title = agent.title,
                description = agent.connection(),
                selected = agent.selected,
                blocked = agent.blocked,
                onSelect = { send(Action.Onboarding(OnboardingAction.ChooseOnboardingAgent(agent.id))) },
            )
        }
    }
}
