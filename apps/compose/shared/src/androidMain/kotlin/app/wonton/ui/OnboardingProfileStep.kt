package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier

import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.OwnerProfile
import app.wonton.protocol.OwnerProfileUpdate
import app.wonton.protocol.StepNav
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.launch

private class OnboardingProfileForm(private val core: ClientCore) {
    var profile by mutableStateOf<OwnerProfile?>(null)
    var name by mutableStateOf("")
    var email by mutableStateOf("")
    var failure by mutableStateOf<CoreException?>(null)
    var saving by mutableStateOf(false)

    suspend fun load() {
        failure = null
        try {
            profile = core.ownerProfile().also {
                name = it.displayName.orEmpty()
                email = it.email.orEmpty()
            }
        } catch (problem: CoreException) {
            failure = problem
        }
    }

    suspend fun save(continueStep: () -> Unit) {
        if (saving) return
        saving = true
        failure = null
        try {
            core.saveOwnerProfile(OwnerProfileUpdate(name, email))
            continueStep()
        } catch (problem: CoreException) {
            failure = problem
        } finally {
            saving = false
        }
    }
}

@Composable
internal fun OnboardingProfileStep(
    view: OnboardingViewModel,
    nav: StepNav,
    core: ClientCore,
    send: (Action) -> Unit,
    modifier: Modifier,
) {
    val form = remember(core) { OnboardingProfileForm(core) }
    var attempt by remember { mutableIntStateOf(0) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(form, attempt) { form.load() }
    StepCardFrame(
        nav = nav,
        title = view.title,
        primaryText = "保存",
        primaryBusy = form.saving || view.loading,
        onBack = { send(Action.BackOnboardingStep) },
        onSkip = { send(Action.SkipOnboardingStep) },
        onPrimary = { scope.launch { form.save { send(Action.ContinueOnboarding) } } },
        modifier = modifier,
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                val problem = view.problem
                when {
                    problem != null -> OnboardingProblem(problem.onboardingTitle(), problem.message) {
                        send(Action.RefreshOnboarding)
                    }
                    else -> {
                        form.failure?.let {
                            val retry: (() -> Unit)? = if (form.profile == null) ({ attempt++ }) else null
                            OnboardingProblem(it.onboardingTitle(), it.toString(), retry)
                        }
                        val profile = form.profile
                        if (profile != null) {
                            OwnerProfileForm(
                                profile, form.name, { form.name = it }, form.email, { form.email = it }, null,
                            )
                        } else if (form.failure == null) {
                            OnboardingLoading()
                        }
                    }
                }
            }
        }
    }
}
