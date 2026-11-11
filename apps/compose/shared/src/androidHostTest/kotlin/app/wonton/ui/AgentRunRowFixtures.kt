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

/**
 * fe-agent-run 的固定假数据：Agent 一轮的十一种状态（ADR-0082）。文字是 Bot 自述的样子，
 * 壳不加工；接线后这份假数据换成核心的 TimelineContent.AgentRun。
 */
object AgentRunRowFixtures {
    val working = AgentRunRowModel(state = AgentRunRowState.Working, step = "Reading files")
    val workingWithProgress = AgentRunRowModel(
        state = AgentRunRowState.Working,
        step = "Running command (step 3)",
        progress = 0.4f,
    )
    val waitingForCard = AgentRunRowModel(
        state = AgentRunRowState.Waiting,
        cardId = "card-1",
        senderName = "Claude Code",
    )
    val waitingWithChoices = AgentRunRowModel(
        state = AgentRunRowState.Waiting,
        senderName = "Hermes",
        choices = listOf(
            AgentRunChoice(key = "yes", label = "Yes, go ahead"),
            AgentRunChoice(key = "no", label = "No, stop"),
        ),
    )
    val doneWithSteps = AgentRunRowModel(state = AgentRunRowState.Done, steps = 5)
    val done = AgentRunRowModel(state = AgentRunRowState.Done)
    val failed = AgentRunRowModel(state = AgentRunRowState.Failed, result = "Model gateway error")
    val cancelled = AgentRunRowModel(state = AgentRunRowState.Failed, end = AgentRunEnd.Cancelled)
    val uncertain = AgentRunRowModel(state = AgentRunRowState.Failed, end = AgentRunEnd.Uncertain)
    val offline = AgentRunRowModel(state = AgentRunRowState.Uncertain)
    val other = AgentRunRowModel(
        state = AgentRunRowState.Other,
        body = "Runner restarted, this run is inconclusive",
    )

    /** 全部状态按顺序排好，供预览和截图测试一次画完。 */
    val all: List<AgentRunRowModel> = listOf(
        working, workingWithProgress, waitingForCard, waitingWithChoices,
        doneWithSteps, done, failed, cancelled, uncertain, offline, other,
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
