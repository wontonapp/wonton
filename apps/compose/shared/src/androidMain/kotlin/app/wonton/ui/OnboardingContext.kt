package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingViewModel
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.toColor

internal class OnboardingContext(
    val view: OnboardingViewModel,
    val chat: ChatViewModel,
    val send: (Action) -> Unit,
)

internal val LocalOnboarding = compositionLocalOf<OnboardingContext> { error("OnboardingContext is not provided") }

@Composable
internal fun OnboardingHost(
    plugins: PluginsState,
    home: HomeViewModel,
    chat: ChatViewModel,
    onFinished: (String?) -> Unit,
    content: @Composable () -> Unit,
) {
    val core = plugins.core
    var skipped by remember(core) { mutableStateOf(false) }
    val send: (Action) -> Unit = { action ->
        if (action == Action.RestartOnboarding) skipped = false
        plugins.send(action)
    }
    val view by produceState(remember(core) { core.onboarding() }, core) {
        core.subscribeOnboarding().use { subscription ->
            while (true) value = subscription.next() ?: break
        }
    }
    val paired = home.status !is HomeStatus.NotConfigured
    OnboardingFinishedEffect(view, paired && !skipped, onFinished)
    CompositionLocalProvider(LocalOnboarding provides OnboardingContext(view, chat, send)) {
        when {
            paired && !skipped && view.blocksHome() -> {
                OnboardingGate(view, plugins, home, send) { skipped = true }
            }
            else -> {
                content()
                if (paired) ReplacedAgentOffer(plugins)
            }
        }
    }
}

private fun OnboardingViewModel.blocksHome(): Boolean =
    step.isOnboardingStep() || (step == null && (loading || problem != null))

@Composable
private fun OnboardingGate(
    view: OnboardingViewModel,
    plugins: PluginsState,
    home: HomeViewModel,
    send: (Action) -> Unit,
    skip: () -> Unit,
) {
    when {
        view.step.isOnboardingStep() -> OnboardingRoute(view, plugins, home)
        view.loading -> {
            BackHandler { }
            Box(
                Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()),
                contentAlignment = Alignment.Center,
            ) { OnboardingLoading() }
        }
        else -> OnboardingGateProblem(view, send, skip)
    }
}

@Composable
private fun OnboardingGateProblem(view: OnboardingViewModel, send: (Action) -> Unit, skip: () -> Unit) {
    BackHandler { }
    Column(
        Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()).safeDrawingPadding()
            .verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16, Alignment.CenterVertically),
    ) {
        view.problem?.let { problem ->
            OnboardingProblem(problem.onboardingTitle(), problem.message) { send(Action.RefreshOnboarding) }
        }
        WontonButton(stringResource(R.string.onboarding_skip_anyway), skip, kind = WontonButtonKind.SECONDARY)
    }
}

@Composable
private fun OnboardingFinishedEffect(view: OnboardingViewModel, paired: Boolean, onFinished: (String?) -> Unit) {
    var previous by remember { mutableStateOf<OnboardingStep?>(null) }
    val finished by rememberUpdatedState(onFinished)
    LaunchedEffect(paired, view.step) {
        if (paired && previous.isOnboardingStep() && view.step == OnboardingStep.FINISHED) {
            finished(view.primaryBotUserId)
        }
        previous = view.step.takeIf { paired }
    }
}

private fun OnboardingStep?.isOnboardingStep(): Boolean = when (this) {
    OnboardingStep.PROFILE, OnboardingStep.AGENT, OnboardingStep.MODEL,
    OnboardingStep.APPS, OnboardingStep.REVIEW -> true
    else -> false
}

@Composable
private fun OnboardingRoute(view: OnboardingViewModel, plugins: PluginsState, home: HomeViewModel) {
    var gateway by remember(view.step) { mutableStateOf<String?>(null) }
    val url = gateway
    if (url != null) {
        OnboardingGatewayScreen(url, home) {
            gateway = null
            plugins.send(Action.RefreshOnboarding)
        }
    } else {
        OnboardingScreen(view, plugins, openGateway = { gateway = it })
    }
}
