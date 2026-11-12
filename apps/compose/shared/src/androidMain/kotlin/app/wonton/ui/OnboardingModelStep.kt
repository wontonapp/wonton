package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.icu.text.ListFormatter
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.protocol.Action
import app.wonton.protocol.ModelProvider
import app.wonton.protocol.ModelProviderFailure
import app.wonton.protocol.ModelProviders
import app.wonton.protocol.OnboardingModelChoice
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun OnboardingModelStep(
    model: app.wonton.protocol.OnboardingModelStep,
    form: OnboardingModelForm,
    send: (Action) -> Unit,
    openGateway: (String) -> Unit,
) {
    val openKeyUrl = rememberKeyBrowser(send)
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        model.problem?.let { problem ->
            key(problem) {
                val retry: (() -> Unit)? = if (problem.reason == ModelProviderFailure.KEY_REJECTED) null
                    else ({ send(Action.RefreshOnboarding) })
                OnboardingProblem(problem.onboardingTitle(), problem.message, retry)
            }
        }
        if (model.providers == null && model.problem == null) OnboardingLoading()
        model.providers?.let { providers ->
            if (providers.configured.isNotEmpty()) {
                val current = ListFormatter.getInstance(LocalConfiguration.current.locales[0])
                    .format(providers.configured.map { it.title })
                Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                    StepOptionRow(
                        title = stringResource(R.string.onboarding_model_use_current),
                        selected = model.choice is OnboardingModelChoice.Current,
                        onSelect = { form.collapse(); send(Action.UseCurrentModel) },
                    )
                    // 网关给的模型名带着内部地址，收进默认收起的“详情”。
                    ProblemDetail(current, Modifier.padding(start = WontonTheme.spacing.s16))
                }
            }
            if (providers.providers.isNullOrEmpty() && providers.webUrl != null) {
                WontonButton(stringResource(R.string.onboarding_model_open_settings),
                    { openGateway(requireNotNull(providers.webUrl)) },
                    kind = WontonButtonKind.SECONDARY)
            } else {
                ModelProviderList(providers, model, form, send, openKeyUrl)
            }
        }
        model.logins.forEach { login ->
            key(login.id) {
                StepOptionRow(
                    title = stringResource(R.string.onboarding_model_use_login, login.title),
                    selected = (model.choice as? OnboardingModelChoice.Login)?.id == login.id,
                    onSelect = { form.collapse(); send(Action.ChooseModelLogin(login.id)) },
                )
            }
        }
    }
}

@Composable
private fun ModelProviderList(
    providers: ModelProviders,
    model: app.wonton.protocol.OnboardingModelStep,
    form: OnboardingModelForm,
    send: (Action) -> Unit,
    openKeyUrl: (String) -> Unit,
) {
    val items = providers.providers.orEmpty()
    var showsMore by remember {
        mutableStateOf(items.any { !it.recommended && it.id == form.expandedProvider })
    }
    items.filter { it.recommended }.forEach { provider ->
        key(provider.id) { ModelProviderRow(provider, model, form, send, openKeyUrl) }
    }
    if (items.any { !it.recommended }) {
        WontonButton(
            stringResource(if (showsMore) R.string.onboarding_model_less else R.string.onboarding_model_more),
            {
                showsMore = !showsMore
                if (!showsMore && items.any { !it.recommended && it.id == form.expandedProvider }) form.collapse()
            },
            kind = WontonButtonKind.SECONDARY,
        )
        if (showsMore) {
            items.filter { !it.recommended }.forEach { provider ->
                key(provider.id) { ModelProviderRow(provider, model, form, send, openKeyUrl) }
            }
        }
    }
    if (providers.custom) {
        val choice = model.choice as? OnboardingModelChoice.Custom
        StepOptionRow(
            title = stringResource(R.string.onboarding_model_custom),
            selected = choice != null,
            onSelect = form::expandCustom,
        )
        if (choice?.reachable == false) UnreachableModelNotice()
        if (form.customExpanded) ModelProviderForm(form, model.testing, send)
    }
}

@Composable
private fun ModelProviderRow(
    provider: ModelProvider,
    model: app.wonton.protocol.OnboardingModelStep,
    form: OnboardingModelForm,
    send: (Action) -> Unit,
    openKeyUrl: (String) -> Unit,
) {
    val choice = (model.choice as? OnboardingModelChoice.Provider)?.takeIf { it.id == provider.id }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        StepOptionRow(
            title = provider.title,
            selected = choice != null,
            onSelect = { form.expandProvider(provider.id) },
        )
        if (choice?.reachable == false) UnreachableModelNotice()
        if (form.expandedProvider == provider.id) {
            ModelProviderForm(form, model.testing, send) {
                provider.keyUrl?.let { url ->
                    WontonButton(stringResource(R.string.onboarding_model_get_key), { openKeyUrl(url) },
                        kind = WontonButtonKind.SECONDARY)
                }
            }
        }
    }
}

@Composable
private fun UnreachableModelNotice() {
    Text(stringResource(R.string.onboarding_model_unreachable), style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.warning.toColor())
}

@Composable
private fun rememberKeyBrowser(send: (Action) -> Unit): (String) -> Unit {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val currentSend by rememberUpdatedState(send)
    var browserOpened by remember { mutableStateOf(false) }
    var browserUnavailable by remember { mutableStateOf(false) }
    DisposableEffect(lifecycle) {
        var leftApp = false
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE && browserOpened) leftApp = true
            if (event == Lifecycle.Event.ON_RESUME && browserOpened && leftApp) {
                browserOpened = false
                leftApp = false
                currentSend(Action.RefreshOnboarding)
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    if (browserUnavailable) {
        WontonNotice(WontonIcon.Warning, stringResource(R.string.onboarding_model_no_browser))
    }
    return { url ->
        try {
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            browserOpened = true
            browserUnavailable = false
        } catch (_: ActivityNotFoundException) {
            browserUnavailable = true
        }
    }
}
