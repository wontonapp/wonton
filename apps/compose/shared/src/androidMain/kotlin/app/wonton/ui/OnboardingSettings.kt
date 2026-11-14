package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource

import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme

@Composable
internal fun DefaultAgentSelection(onDismiss: () -> Unit) {
    val context = LocalOnboarding.current
    WontonDialog(
        stringResource(R.string.onboarding_default_agent_title),
        onDismiss = onDismiss,
        actions = listOf(WontonDialogAction(stringResource(R.string.common_close), onClick = onDismiss)),
        detail = {
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                context.view.agents.forEach { agent ->
                    StepOptionRow(
                        title = agent.title,
                        description = agent.connection(),
                        selected = agent.selected,
                        blocked = agent.blocked,
                        onSelect = { context.send(Action.Onboarding(OnboardingAction.SetPrimaryAgent(agent.id))) },
                    )
                }
                if (context.view.agents.isEmpty()) {
                    OnboardingProblem(stringResource(R.string.onboarding_agent_empty), null) {
                        context.send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
                    }
                }
            }
        },
    )
}
