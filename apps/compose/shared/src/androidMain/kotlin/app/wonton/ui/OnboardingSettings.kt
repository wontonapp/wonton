package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue

import app.wonton.protocol.Action
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme

@Composable
internal fun OnboardingSettingsRows() {
    val context = LocalOnboarding.current
    var choosing by remember { mutableStateOf(false) }
    WontonGroup {
        WontonRow("重看引导", onClick = { context.send(Action.RestartOnboarding) })
        WontonSeparator()
        WontonRow(
            "默认 Agent",
            value = context.view.agents.firstOrNull { it.selected }?.title,
            showsForward = true,
            onClick = { choosing = true },
        )
    }
    context.view.problem?.let { problem ->
        OnboardingProblem(problem.onboardingTitle(), problem.message) { context.send(Action.RefreshOnboarding) }
    }
    if (choosing) {
        WontonDialog(
            "默认 Agent",
            onDismiss = { choosing = false },
            actions = listOf(WontonDialogAction("关闭") { choosing = false }),
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
}
