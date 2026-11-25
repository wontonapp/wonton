package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.ChoiceSend
import app.wonton.protocol.ChoicesState
import app.wonton.protocol.MessageChoice
import app.wonton.protocol.MessageChoices
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 一行两个。 */
private const val PAIR = 2

/**
 * 消息下面的选项按钮（ADR-0109）：两个一行、单独一个占满整行，跟气泡一样宽。选项还开着时点了就把 key 交给核心；
 * 回答过以后选中的那项一直高亮、其余变灰；这条不是问我的就不能点。点过还没答上的转圈，答不上时下面一行红字说没发出去、
 * 再点一次重试，核心的原文收进可展开的“详情”（ADR-0109 §7）。按钮上先画核心给的 key（表情），再画它的说明；没有说明时只画 key。
 * 两者同一行居中，放不下省略。
 */
@Composable
internal fun MessageChoicesRow(choices: MessageChoices, onAnswer: (String) -> Unit) {
    val failed = choices.options.firstNotNullOfOrNull { (it.send as? ChoiceSend.Failed)?.message }
    Column(
        Modifier.fillMaxWidth().padding(top = WontonTheme.spacing.s4).testTag("choices-row"),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        choices.options.chunked(PAIR).forEach { pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                if (pair.size == 1) {
                    ChoiceButton(pair[0], choices.state, Modifier.fillMaxWidth(), onAnswer)
                } else {
                    pair.forEach { choice -> ChoiceButton(choice, choices.state, Modifier.weight(1f), onAnswer) }
                }
            }
        }
        failed?.let {
            Text(
                stringResource(R.string.message_choice_failed),
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.danger.toColor(),
            )
            ProblemDetail(it)
        }
    }
}

@Composable
private fun RowScope.ChoiceButton(
    choice: MessageChoice,
    state: ChoicesState,
    modifier: Modifier,
    onAnswer: (String) -> Unit,
) {
    val send = choice.send
    val open = state == ChoicesState.OPEN && send !is ChoiceSend.Sending
    StepOptionRow(
        title = choice.label ?: choice.key,
        leading = if (choice.label == null) {
            {}
        } else {
            { Text(choice.key, style = WontonTheme.typography.body) }
        },
        selected = choice.chosen,
        onSelect = if (open) {
            { onAnswer(choice.key) }
        } else {
            null
        },
        modifier = modifier.testTag("choice-${choice.key}"),
        // 表情和字并排居中、单行省略：两个并排的窄按钮里折行会把 key 和说明拆开。
        centered = true,
        trailing = if (send is ChoiceSend.Sending) {
            {
                Box(Modifier.padding(end = WontonTheme.spacing.s12)) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.iconSmall),
                        strokeWidth = WontonTheme.size.hairline,
                    )
                }
            }
        } else {
            {}
        },
    )
}
