package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Answer
import app.wonton.protocol.CardChoice
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView

/** 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConfirmationCardView(view: ConfirmationView, onAnswer: (String, Answer) -> Unit) {
    val card = view.card
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(card.title, style = MaterialTheme.typography.titleSmall)
        card.lines.forEach { line ->
            Text(
                line.text,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = if (line.emphasis) FontWeight.SemiBold else FontWeight.Normal,
                color = if (line.emphasis) MaterialTheme.colorScheme.error else Color.Unspecified,
            )
        }
        CardState(card.state)
        view.problem?.let { problem ->
            Text(problem.message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
        if (card.choices.isNotEmpty()) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                card.choices.forEach { choice ->
                    ChoiceButton(choice, view.sending) { onAnswer(card.id, choice.answer) }
                }
            }
        }
    }
}

@Composable
private fun CardState(state: ConfirmationState) {
    val style = MaterialTheme.typography.bodySmall
    val error = MaterialTheme.colorScheme.error
    when (state) {
        ConfirmationState.Pending -> Unit
        ConfirmationState.Running -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
            Text("正在执行", style = style)
        }
        is ConfirmationState.Succeeded -> Text(state.message, style = style, color = MaterialTheme.colorScheme.primary)
        is ConfirmationState.Failed -> Text("没有成功：${state.message}", style = style, color = error)
        is ConfirmationState.Uncertain ->
            Text("结果不确定：${state.message}", style = style, color = MaterialTheme.colorScheme.tertiary)
        ConfirmationState.Rejected -> Text("已拒绝，没有执行", style = style)
        ConfirmationState.Expired -> Text("已过期，没有执行", style = style)
    }
}

@Composable
private fun ChoiceButton(choice: CardChoice, sending: Answer?, onClick: () -> Unit) {
    val label: @Composable () -> Unit = {
        if (sending == choice.answer) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
        }
        Text(choice.label)
    }
    if (choice.answer == Answer.APPROVE) {
        Button(onClick, enabled = sending == null) { label() }
    } else {
        OutlinedButton(onClick, enabled = sending == null) { label() }
    }
}
