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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.protocol.ThemeColors
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Agent
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.Notify
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.WontonIcon

/** Agent 一轮的状态（ADR-0082 的 `state`）：`Uncertain` 由核心合成，Bot 离线时不转圈只说不确定；不认识的值落到 `Other`。 */
enum class AgentRunRowState { Working, Waiting, Done, Failed, Uncertain, Other }

/** 这一轮怎么结束的（`app.wonton.end`）。`Other` 的原文只在 `result` 里，壳不单独画。 */
enum class AgentRunEnd { Cancelled, Uncertain }

/** Agent 自己的提问（MSC4564 的 `choices`）：以主人的身份发反应作答，不经核心确认。 */
data class AgentRunChoice(val key: String, val label: String)

/**
 * 时间线里 Agent 一轮运行状态那一行的数据（ADR-0082）。接线时由调用处从核心的
 * `TimelineContent.AgentRun` 映射。`step`、`result`、`body`、choice 的 `label` 是 Bot 自己写的
 * 文字，原样显示，不翻译、不拼句。
 */
data class AgentRunRowModel(
    val state: AgentRunRowState,
    /** 当前步骤的一句自述。 */
    val step: String? = null,
    /** 到现在几步。 */
    val steps: Int? = null,
    /** 0～1，有计划时才有。 */
    val progress: Float? = null,
    /** 结束时的一句总结。 */
    val result: String? = null,
    /** 打断了、结果不确定这两种结束的区分。 */
    val end: AgentRunEnd? = null,
    /** `Other` 状态按普通 Bot 提示消息显示的正文。 */
    val body: String? = null,
    /** waiting 且这个 Bot 在这个房间有等回答的核心卡片：整行可点，打开那张卡片。 */
    val cardId: String? = null,
    /** Bot 自己的提问。 */
    val choices: List<AgentRunChoice> = emptyList(),
    /** 发这条状态的 Bot 的名字，choices 区标明是谁在问。 */
    val senderName: String = "",
)

/**
 * 时间线里 Agent 一轮运行状态的那一行（ADR-0082）：真的在干活才转圈，要人管的醒目，
 * 打断了和结果不确定不用失败的红。减弱动态效果时不转圈。
 */
@Composable
fun AgentRunRowView(
    run: AgentRunRowModel,
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
        StatusLine(run, onOpenCard)
        if (run.state == AgentRunRowState.Working && run.progress != null) {
            LinearProgressIndicator(
                progress = { run.progress ?: 0f },
                Modifier.fillMaxWidth(),
                color = WontonTheme.colors.accentInk.toColor(),
            )
        }
        if (run.choices.isNotEmpty()) {
            Asking(run, onAnswerChoice)
        }
    }
}

/** waiting 且有等回答的核心卡片时整行可点（ADR-0043：能跳转的地方方便跳转）。 */
@Composable
private fun StatusLine(run: AgentRunRowModel, onOpenCard: (String) -> Unit) {
    val colors = WontonTheme.colors
    val cardId = run.cardId?.takeIf { run.state == AgentRunRowState.Waiting }
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
                style = if (run.state == AgentRunRowState.Done) {
                    WontonTheme.typography.caption
                } else {
                    WontonTheme.typography.supporting
                },
                color = statusInk(run, colors).toColor(),
            )
        }
        if (cardId != null) {
            // 装饰：整行可点，箭头只表示能过去。
            Icon(WontonIcon.Forward, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.accentInk.toColor())
        }
    }
}

/** 真的在干活才转圈；减弱动态效果时不转圈，换成静态的 Agent 图标。 */
@Composable
private fun Mark(run: AgentRunRowModel, colors: ThemeColors) {
    val tint = statusInk(run, colors).toColor()
    when (run.state) {
        AgentRunRowState.Working ->
            if (AvatarEnvironment.reduceMotion) {
                Icon(WontonIcon.Agent, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
            } else {
                CircularProgressIndicator(
                    Modifier.size(WontonTheme.size.iconSmall),
                    color = colors.accentInk.toColor(),
                )
            }
        AgentRunRowState.Waiting ->
            Icon(WontonIcon.Notify, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        AgentRunRowState.Done ->
            Icon(WontonIcon.CheckCircleFill, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        AgentRunRowState.Failed -> when (run.end) {
            AgentRunEnd.Cancelled ->
                Icon(WontonIcon.PauseFill, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
            AgentRunEnd.Uncertain ->
                Icon(WontonIcon.Help, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
            null ->
                Icon(WontonIcon.Error, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        }
        AgentRunRowState.Uncertain ->
            Icon(WontonIcon.Help, null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        AgentRunRowState.Other -> Unit
    }
}

/** 状态行的文字。打断了、结果不确定用壳自己的文字，其余显示 Bot 的自述。 */
@Composable
private fun statusText(run: AgentRunRowModel): String = when (run.state) {
    AgentRunRowState.Working -> run.step ?: ""
    AgentRunRowState.Waiting -> stringResource(R.string.agent_run_waiting)
    AgentRunRowState.Done ->
        if (run.steps != null) {
            pluralStringResource(R.plurals.agent_run_done_steps, run.steps, run.steps)
        } else {
            stringResource(R.string.agent_run_done)
        }
    AgentRunRowState.Failed -> when (run.end) {
        AgentRunEnd.Cancelled -> stringResource(R.string.agent_run_cancelled)
        AgentRunEnd.Uncertain -> stringResource(R.string.agent_run_uncertain)
        null -> run.result ?: ""
    }
    AgentRunRowState.Uncertain -> stringResource(R.string.agent_run_uncertain)
    AgentRunRowState.Other -> run.body ?: ""
}

private fun statusInk(run: AgentRunRowModel, colors: ThemeColors) = when (run.state) {
    AgentRunRowState.Working, AgentRunRowState.Other -> colors.textSecondary
    AgentRunRowState.Waiting -> colors.accentInk
    AgentRunRowState.Done -> colors.success
    AgentRunRowState.Failed -> when (run.end) {
        AgentRunEnd.Cancelled -> colors.textSecondary
        AgentRunEnd.Uncertain -> colors.warning
        null -> colors.danger
    }
    AgentRunRowState.Uncertain -> colors.warning
}

/** choices 区：来源行标明是这个 Bot 在问，按钮是描边胶囊——和核心确认卡片的实心按钮分开。 */
@Composable
private fun Asking(run: AgentRunRowModel, onAnswerChoice: (String) -> Unit) {
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
