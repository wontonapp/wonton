package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.Answer
import app.wonton.protocol.CardChoice
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.WindowView
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date

/** 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConfirmationCardView(view: ConfirmationView, onAction: (Action) -> Unit) {
    val card = view.card
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(card.title, style = WontonTheme.typography.itemTitle)
        card.lines.forEach { line ->
            Text(
                line.text,
                style = WontonTheme.typography.supporting,
                fontWeight = if (line.emphasis) FontWeight.SemiBold else FontWeight.Normal,
                color = (if (line.emphasis) colors.danger else colors.textPrimary).toColor(),
            )
        }
        CardState(card.state)
        view.window?.let { window -> CardWindow(window) { onAction(Action.StopConfirmationWindow(card.id)) } }
        view.problem?.let { problem -> StateLine(WontonIcon.Warning, problem.message, colors.danger) }
        if (card.choices.isNotEmpty()) {
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                card.choices.forEach { choice ->
                    ChoiceButton(choice, view) {
                        onAction(Action.AnswerConfirmation(card.id, choice.answer, choice.choiceId))
                    }
                }
            }
        }
    }
}

@Composable
private fun CardState(state: ConfirmationState) {
    val style = WontonTheme.typography.supporting
    val colors = WontonTheme.colors
    when (state) {
        ConfirmationState.Pending -> Unit
        ConfirmationState.Running -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
            Text("正在执行", style = style)
        }
        is ConfirmationState.Succeeded -> StateLine(WontonIcon.CheckCircleFill, state.message, colors.success)
        is ConfirmationState.Failed -> StateLine(WontonIcon.Error, "没有成功：${state.message}", colors.danger)
        is ConfirmationState.Uncertain ->
            StateLine(WontonIcon.Help, "结果不确定：${state.message}", colors.warning)
        ConfirmationState.Rejected -> Text("已拒绝，没有执行", style = style, color = colors.textSecondary.toColor())
        ConfirmationState.Expired -> Text("已过期，没有执行", style = style, color = colors.textSecondary.toColor())
    }
}

@Composable
private fun CardWindow(window: WindowView, onStop: () -> Unit) {
    val style = WontonTheme.typography.supporting
    val colors = WontonTheme.colors
    when (window) {
        is WindowView.Ended -> Text(window.note, style = style, color = colors.textSecondary.toColor())
        is WindowView.Active -> {
            val until = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(window.expiresAtMs.toLong()))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Text("放行到 $until", style = style)
                if (window.stopping) {
                    CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
                }
                WontonButton("停止", onStop, kind = WontonButtonKind.SECONDARY, enabled = !window.stopping)
            }
            window.problem?.let { problem -> StateLine(WontonIcon.Warning, problem.message, colors.danger) }
        }
    }
}

@Composable
private fun ChoiceButton(choice: CardChoice, view: ConfirmationView, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        if (view.sending == choice.answer && view.sendingChoiceId == choice.choiceId) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
        }
        WontonButton(
            choice.label,
            onClick,
            kind = if (choice.answer == Answer.APPROVE) WontonButtonKind.PRIMARY else WontonButtonKind.SECONDARY,
            enabled = view.sending == null,
        )
    }
}
