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
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingCommit
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StepNav
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun OnboardingScreen(
    view: OnboardingViewModel,
    plugins: PluginsState,
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
    // 前三步各有自己的输入或子页，自己画卡片。
    when (view.step) {
        OnboardingStep.PROFILE -> OnboardingProfileStep(view, nav, send, modifier)
        OnboardingStep.ASSISTANT -> OnboardingAssistantStep(view, nav, send, onboarding.close, modifier)
        OnboardingStep.MODEL -> OnboardingModelStep(view, nav, plugins, onboarding.close, modifier)
        else -> OnboardingStepCard(view, nav, plugins, modifier)
    }
}

@Composable
private fun OnboardingStepCard(view: OnboardingViewModel, nav: StepNav, plugins: PluginsState, modifier: Modifier) {
    val send = plugins::send
    val onboarding = LocalOnboarding.current
    val scroll = rememberScrollState()
    LaunchedEffect(view.step) { scroll.scrollTo(0) }
    LaunchedEffect(view.problem) {
        if (view.problem != null) scroll.scrollTo(0)
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
    OnboardingStep.ASSISTANT -> stringResource(R.string.onboarding_step_assistant)
    OnboardingStep.MODEL -> stringResource(R.string.onboarding_step_model)
    OnboardingStep.APPS -> stringResource(R.string.onboarding_step_apps)
    OnboardingStep.REVIEW -> stringResource(R.string.onboarding_step_review)
    OnboardingStep.FINISHED -> stringResource(R.string.onboarding_step_finished)
    OnboardingStep.OTHER -> stringResource(R.string.onboarding_step_other)
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
