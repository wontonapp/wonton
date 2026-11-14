package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource

import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StepNav
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTheme

@Composable
internal fun OnboardingProfileStep(
    view: OnboardingViewModel,
    nav: StepNav,
    send: (Action) -> Unit,
    modifier: Modifier,
) {
    var name by remember { mutableStateOf(view.profile.displayName) }
    var email by remember { mutableStateOf(view.profile.email) }
    val onboarding = LocalOnboarding.current
    StepCardFrame(
        nav = nav,
        title = view.title(),
        primaryText = stringResource(R.string.onboarding_next),
        primaryBusy = view.loading,
        onBack = { send(Action.Onboarding(OnboardingAction.BackOnboardingStep)) },
        onSkip = { send(Action.Onboarding(OnboardingAction.SkipOnboardingStep)) },
        onPrimary = { send(Action.Onboarding(OnboardingAction.ContinueOnboarding)) },
        onClose = onboarding.close,
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
                        send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                    }
                    else -> {
                        OwnerProfileFields(
                            name = name,
                            onName = {
                                name = it
                                send(Action.Onboarding(OnboardingAction.EditOnboardingProfile(name, email)))
                            },
                            email = email,
                            onEmail = {
                                email = it
                                send(Action.Onboarding(OnboardingAction.EditOnboardingProfile(name, email)))
                            },
                            nameProblem = view.profile.displayNameProblem,
                            emailProblem = view.profile.emailProblem,
                        )
                    }
                }
            }
        }
    }
}
