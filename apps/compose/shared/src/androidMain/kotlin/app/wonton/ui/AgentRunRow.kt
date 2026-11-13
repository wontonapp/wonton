package app.wonton.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.protocol.AgentRunEnd
import app.wonton.protocol.AgentRunState
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.TimelineContent
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Agent
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.Notify
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.WontonIcon

/**
 * 时间线里 Agent 一轮运行状态的那一行（ADR-0082）。数据直接用核心的 `TimelineContent.AgentRun`：
 * 真的在干活才转圈，要人管的醒目，打断了和结果不确定不用失败的红；减弱动态效果时不转圈。
 * `step`、`result`、`body`、choice 的 `label` 是 Bot 自己写的文字，原样显示，不翻译、不拼句；
 * 不认识的 `state` 落到通用样子，按 `body` 显示。
 * 做完了的一轮只留一行小的步骤摘要，点开是 Bot 自己的总结；没有步数也没有总结的不占地方（[isBlank]）。
 */
@Composable
fun AgentRunRowView(
    run: TimelineContent.AgentRun,
    onOpenCard: (String) -> Unit,
    onAnswerChoice: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    // 减弱动态效果是进程级监听（ADR-0044），这里补一次注册，取到的是实时值。
    AvatarEnvironment.observeSystem(LocalContext.current)
    Column(
        modifier.fillMaxWidth().padding(end = WontonTheme.spacing.s48),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (run.state == AgentRunState.Done) DoneSummary(run) else StatusLine(run, onOpenCard)
        if (run.state == AgentRunState.Working && run.progress != null) {
            LinearProgressIndicator(
                progress = { run.progress },
                Modifier.fillMaxWidth(),
                color = WontonTheme.colors.accentInk.toColor(),
            )
        }
        if (run.state !is AgentRunState.Other && run.choices.isNotEmpty()) {
            Asking(run, onAnswerChoice)
        }
    }
}

/** 做完了、没有步数也没有总结、也没在问的一轮没有可写的：时间线不给它留行。 */
val TimelineContent.AgentRun.isBlank: Boolean
    get() = state == AgentRunState.Done && (steps ?: 0u) == 0u && result.isNullOrEmpty() && choices.isEmpty()

/** 展开时尖角朝下：转四分之一圈。 */
private const val EXPANDED_TURN = 90f

/** 做完的一轮：一行“N 步”，有总结时能点开；没有步数只有总结时直接写总结。 */
@Composable
private fun DoneSummary(run: TimelineContent.AgentRun) {
    val ink = WontonTheme.colors.textSecondary.toColor()
    val style = WontonTheme.typography.caption
    val total = run.steps?.toInt() ?: 0
    val result = run.result.orEmpty()
    var showsResult by remember { mutableStateOf(false) }
    if (total > 0) {
        Row(
            Modifier
                .then(
                    if (result.isEmpty()) {
                        Modifier
                    } else {
                        Modifier
                            .testTag("agentRunSteps")
                            .clickable(role = Role.Button) { showsResult = !showsResult }
                            .heightIn(min = WontonTheme.size.chip)
                    },
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Text(pluralStringResource(R.plurals.agent_run_steps, total, total), style = style, color = ink)
            if (result.isNotEmpty()) {
                // 装饰：整行可点，尖角只表示能展开。
                Icon(
                    WontonIcon.Forward,
                    null,
                    Modifier.size(WontonTheme.size.iconTiny).rotate(if (showsResult) EXPANDED_TURN else 0f),
                    tint = ink,
                )
            }
        }
    }
    if (result.isNotEmpty() && (showsResult || total == 0)) {
        Text(result, style = style, color = ink)
    }
}

/** waiting 且有等回答的核心卡片时整行可点（ADR-0043：能跳转的地方方便跳转）。 */
@Composable
private fun StatusLine(run: TimelineContent.AgentRun, onOpenCard: (String) -> Unit) {
    val colors = WontonTheme.colors
    val cardId = run.cardId?.takeIf { run.state == AgentRunState.Waiting }
    Row(
        Modifier.fillMaxWidth()
            .then(
                if (cardId != null) {
                    Modifier.clickable(role = Role.Button) { onOpenCard(cardId) }
                        .heightIn(min = WontonTheme.size.target)
                } else {
                    Modifier
                },
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Mark(run, colors)
        val text = statusText(run)
        if (text.isNotEmpty()) {
            Text(
                text,
                Modifier.weight(1f),
                style = WontonTheme.typography.supporting,
                color = statusInk(run, colors).toColor(),
            )
        }
        if (cardId != null) {
            // 装饰：整行可点，箭头只表示能过去。
            Icon(WontonIcon.Forward, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.accentInk.toColor())
        }
    }
}

/**
 * 真的在干活才转圈；减弱动态效果时不转圈，换成静态的 Agent 图标。
 * 不认识的状态不画标记。
 */
@Composable
private fun Mark(run: TimelineContent.AgentRun, colors: ThemeColors) {
    val tint = statusInk(run, colors).toColor()
    when (run.state) {
        AgentRunState.Working ->
            if (AvatarEnvironment.reduceMotion) {
                Icon(WontonIcon.Agent, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
            } else {
                CircularProgressIndicator(
                    Modifier.size(WontonTheme.size.iconSmall).testTag("agentRunSpinner"),
                    color = colors.accentInk.toColor(),
                )
            }
        AgentRunState.Waiting ->
            Icon(WontonIcon.Notify, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        AgentRunState.Failed -> failedMark(run, colors)
        AgentRunState.Uncertain ->
            Icon(WontonIcon.Help, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        AgentRunState.Done, is AgentRunState.Other -> Unit
    }
}

/** 失败按结束原因分开：打断和不确定不用失败的红，以后新加的结束原因按普通失败画。 */
@Composable
private fun failedMark(run: TimelineContent.AgentRun, colors: ThemeColors) {
    val (icon, tint) = when (run.end) {
        AgentRunEnd.Cancelled -> WontonIcon.PauseFill to colors.textSecondary.toColor()
        AgentRunEnd.Uncertain -> WontonIcon.Help to colors.warning.toColor()
        is AgentRunEnd.Other, null -> WontonIcon.Error to colors.danger.toColor()
    }
    Icon(icon, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
}

/** 状态行的文字。打断了、结果不确定用壳自己的文字，其余显示 Bot 的自述。 */
@Composable
private fun statusText(run: TimelineContent.AgentRun): String = when (run.state) {
    AgentRunState.Working -> run.step ?: ""
    AgentRunState.Waiting -> stringResource(R.string.agent_run_waiting)
    AgentRunState.Done -> ""
    AgentRunState.Failed -> when {
        run.end is AgentRunEnd.Cancelled -> stringResource(R.string.agent_run_cancelled)
        run.end is AgentRunEnd.Uncertain -> stringResource(R.string.agent_run_uncertain)
        else -> run.result ?: ""
    }
    AgentRunState.Uncertain -> stringResource(R.string.agent_run_uncertain)
    is AgentRunState.Other -> run.body
}

private fun statusInk(run: TimelineContent.AgentRun, colors: ThemeColors) = when (run.state) {
    AgentRunState.Working -> colors.textSecondary
    AgentRunState.Waiting -> colors.accentInk
    AgentRunState.Done -> colors.textSecondary
    AgentRunState.Failed -> when {
        run.end is AgentRunEnd.Cancelled -> colors.textSecondary
        run.end is AgentRunEnd.Uncertain -> colors.warning
        else -> colors.danger
    }
    AgentRunState.Uncertain -> colors.warning
    is AgentRunState.Other -> colors.textSecondary
}

/** choices 区：来源行标明是这个 Bot 在问，按钮是描边胶囊——和核心确认卡片的实心按钮分开。 */
@Composable
private fun Asking(run: TimelineContent.AgentRun, onAnswerChoice: (String) -> Unit) {
    val colors = WontonTheme.colors
    val ink = colors.accentInk.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            Icon(WontonIcon.Agent, null, Modifier.size(WontonTheme.size.iconSmall), tint = ink)
            Text(
                stringResource(R.string.agent_run_asking, run.senderName),
                style = WontonTheme.typography.supporting,
                color = ink,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            run.choices.forEach { choice ->
                Row(
                    Modifier
                        .border(
                            width = WontonTheme.size.hairline,
                            color = ink,
                            shape = RoundedCornerShape(WontonTheme.radii.pill),
                        )
                        .clickable(role = Role.Button) { onAnswerChoice(choice.key) }
                        .heightIn(min = WontonTheme.size.target)
                        .padding(horizontal = WontonTheme.spacing.s12),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(choice.label, style = WontonTheme.typography.supporting, color = ink)
                }
            }
        }
    }
}
