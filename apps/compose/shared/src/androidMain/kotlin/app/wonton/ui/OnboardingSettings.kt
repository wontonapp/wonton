package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable

import app.wonton.protocol.Action
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme

@Composable
internal fun DefaultAgentSelection(onDismiss: () -> Unit) {
    val context = LocalOnboarding.current
    WontonDialog(
        "默认 Agent",
        onDismiss = onDismiss,
        actions = listOf(WontonDialogAction("关闭", onClick = onDismiss)),
        detail = {
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                context.view.agents.forEach { agent ->
                    StepOptionRow(
                        title = agent.title,
                        description = agent.connection,
                        selected = agent.selected,
                        blocked = agent.blocked,
                        onSelect = { context.send(Action.SetPrimaryAgent(agent.id)) },
                    )
                }
                if (context.view.agents.isEmpty()) {
                    OnboardingProblem("没有可选的 Agent，可以刷新重试。", null) {
                        context.send(Action.RefreshOnboarding)
                    }
                }
            }
        },
    )
}
