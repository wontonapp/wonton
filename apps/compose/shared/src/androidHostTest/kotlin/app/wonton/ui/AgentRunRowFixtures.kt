@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AgentRunChoice
import app.wonton.protocol.AgentRunEnd
import app.wonton.protocol.AgentRunState
import app.wonton.protocol.TimelineContent

/**
 * 状态行的固定假数据：Agent 一轮的各种状态（ADR-0082）。只在预览和截图测试里用，
 * 字段和取值都是核心的 `AgentRunState`、`AgentRunEnd`、`AgentRunChoice`，不复刻壳的模型。
 */
object AgentRunRowFixtures {
    private fun row(
        state: AgentRunState,
        step: String? = null,
        steps: UInt? = null,
        progress: Float? = null,
        result: String? = null,
        end: AgentRunEnd? = null,
        body: String = "",
        cardId: String? = null,
        choices: List<AgentRunChoice> = emptyList(),
        senderName: String = "",
    ) = TimelineContent.AgentRun(
        run = null,
        state = state,
        step = step,
        steps = steps,
        progress = progress,
        result = result,
        end = end,
        body = body,
        cardId = cardId,
        choices = choices,
        senderName = senderName,
    )

    val working = row(AgentRunState.Working, step = "Reading files")
    val workingWithProgress = row(
        AgentRunState.Working,
        step = "Running command (step 3)",
        progress = 0.4f,
    )
    val waitingForCard = row(AgentRunState.Waiting, cardId = "card-1", senderName = "Claude Code")
    val waitingWithChoices = row(
        AgentRunState.Waiting,
        senderName = "Hermes",
        choices = listOf(
            AgentRunChoice(key = "yes", label = "Yes, go ahead"),
            AgentRunChoice(key = "no", label = "No, stop"),
        ),
    )
    val doneWithSteps = row(AgentRunState.Done, steps = 5u)
    val doneWithSummary = row(
        AgentRunState.Done,
        steps = 3u,
        result = "Updated the config and restarted the service",
    )
    val done = row(AgentRunState.Done)
    val failed = row(AgentRunState.Failed, result = "Model gateway error")
    val cancelled = row(AgentRunState.Failed, end = AgentRunEnd.Cancelled)
    val uncertain = row(AgentRunState.Failed, end = AgentRunEnd.Uncertain)
    val offline = row(AgentRunState.Uncertain)
    val other = row(
        AgentRunState.Other("new-state"),
        body = "Runner restarted, this run is inconclusive",
    )

    /** 全部状态按顺序排好，供预览和截图测试一次画完。 */
    val all: List<TimelineContent.AgentRun> = listOf(
        working, workingWithProgress, waitingForCard, waitingWithChoices,
        doneWithSteps, doneWithSummary, done, failed, cancelled, uncertain, offline, other,
    )
}

/** 一列画完全部状态：状态之间的颜色、图标和排版差异就是参考图要看的东西。 */
@Composable
internal fun AgentRunRowStatesPreview() {
    PreviewFrame {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            AgentRunRowFixtures.all.forEach { run ->
                AgentRunRowView(run, onOpenCard = {}, onAnswerChoice = {})
            }
        }
    }
}

@Preview(name = "Agent 运行状态", locale = "en", widthDp = 390, heightDp = 720)
@Composable
internal fun AgentRunRowAllStatesPreview() {
    AgentRunRowStatesPreview()
}
