package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StepNav
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 你的助手：给默认 Bot 起名字（ADR-0129 第 5 节），预填核心给的名字；只记进引导草稿，点完成才保存。 */
@Composable
fun OnboardingAssistantStep(
    view: OnboardingViewModel,
    nav: StepNav,
    send: (Action) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
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
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                val problem = view.problem
                if (problem != null) {
                    OnboardingProblem(problem.onboardingTitle(), problem.message) {
                        send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                    }
                    return@Column
                }
                // 名字晚到：等核心取回来再摆输入框，免得记住空串。
                if (view.loading) {
                    OnboardingLoading()
                    return@Column
                }
                // 输入法组字时文字要立刻回到输入框；核心拿到的是同一份。
                var name by remember { mutableStateOf(view.assistant.name) }
                WontonTextField(
                    name,
                    {
                        name = it
                        send(Action.Onboarding(OnboardingAction.EditOnboardingAssistant(it)))
                    },
                    Modifier.fillMaxWidth(),
                    label = stringResource(R.string.bot_name_title),
                    placeholder = stringResource(R.string.bot_name_title),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Words),
                )
                view.assistant.nameProblem?.let {
                    Text(
                        stringResource(it.textRes()),
                        style = WontonTheme.typography.supporting, color = WontonTheme.colors.danger.toColor(),
                    )
                }
            }
        }
    }
}
