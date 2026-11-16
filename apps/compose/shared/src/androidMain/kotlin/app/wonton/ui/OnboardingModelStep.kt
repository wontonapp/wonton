package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingModelChoice
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme

@Composable
internal fun OnboardingModelStep(
    model: app.wonton.protocol.OnboardingModelStep,
    send: (Action) -> Unit,
    openGateway: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        model.problem?.let { problem ->
            key(problem) {
                OnboardingProblem(problem.onboardingTitle(), problem.message) {
                    send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                }
            }
        }
        if (model.settings == null && model.problem == null) OnboardingLoading()
        if (model.hasConfiguredModels) {
            StepOptionRow(
                title = stringResource(R.string.onboarding_model_use_current),
                selected = model.choice is OnboardingModelChoice.Current,
                onSelect = { send(Action.Onboarding(OnboardingAction.UseCurrentModel)) },
            )
        }
        model.settings?.let { settings ->
            WontonButton(
                stringResource(R.string.onboarding_model_open_settings),
                { openGateway(settings.webUrl) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
        model.logins.forEach { login ->
            key(login.id) {
                StepOptionRow(
                    title = stringResource(R.string.onboarding_model_use_login, login.title),
                    selected = (model.choice as? OnboardingModelChoice.Login)?.id == login.id,
                    onSelect = { send(Action.Onboarding(OnboardingAction.ChooseModelLogin(login.id))) },
                )
            }
        }
    }
}
