package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.AgentToolCall
import app.wonton.protocol.AgentToolKind
import app.wonton.protocol.AgentToolStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Agent
import app.wonton.ui.icons.Code
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Document
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Folder
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Globe
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.Search
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.icons.Wrench

/** 展开时尖角朝下：转四分之一圈。 */
private const val EXPANDED_TURN = 90f

/** 一项说明最多占几行：Bot 给的预览本来就是一行，窄屏上折一次。 */
private const val TITLE_LINES = 2

/**
 * 状态行下面那一行“N 步”（ADR-0128 第 4 节）：有清单或总结可看时能点，点了展开或收起；默认收起。
 * 没有可看的就只是一行字。
 */
@Composable
internal fun AgentRunSteps(total: Int, expandable: Boolean, expanded: Boolean, onToggle: () -> Unit) {
    val ink = WontonTheme.colors.textSecondary.toColor()
    Row(
        if (expandable) {
            Modifier
                .testTag("agentRunSteps")
                .clickable(role = Role.Button, onClick = onToggle)
                .heightIn(min = WontonTheme.size.chip)
        } else {
            Modifier
        },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Text(
            pluralStringResource(R.plurals.agent_run_steps, total, total),
            style = WontonTheme.typography.caption,
            color = ink,
        )
        if (expandable) {
            // 装饰：整行可点，尖角只表示能展开。
            Icon(
                WontonIcon.Forward,
                null,
                Modifier.size(WontonTheme.size.iconTiny).rotate(if (expanded) EXPANDED_TURN else 0f),
                tint = ink,
            )
        }
    }
}

/**
 * 这一轮调过的工具，一项一行：按类别的图标加 Bot 写的说明（原样显示，不翻译）。进行中小转圈（减弱动态效果时不转），
 * 失败用错误色的标记，结果不确定用不确定的标记，完成不加标记。
 */
@Composable
internal fun AgentRunTrace(calls: List<AgentToolCall>) {
    if (calls.isEmpty()) return
    Column(Modifier.testTag("agentRunTrace"), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        calls.forEach { call -> TraceRow(call) }
    }
}

@Composable
private fun TraceRow(call: AgentToolCall) {
    val colors = WontonTheme.colors
    val ink = colors.textSecondary.toColor()
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(kindIcon(call.kind), null, Modifier.size(WontonTheme.size.iconTiny), tint = ink)
        Text(
            call.title,
            Modifier.weight(1f, fill = false),
            style = WontonTheme.typography.caption,
            color = ink,
            maxLines = TITLE_LINES,
            overflow = TextOverflow.Ellipsis,
        )
        when (call.status) {
            AgentToolStatus.InProgress ->
                if (!AvatarEnvironment.reduceMotion) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.iconTiny),
                        color = colors.accentInk.toColor(),
                    )
                }
            AgentToolStatus.Failed ->
                Icon(WontonIcon.Error, null, Modifier.size(WontonTheme.size.iconTiny), tint = colors.danger.toColor())
            AgentToolStatus.Uncertain ->
                Icon(WontonIcon.Help, null, Modifier.size(WontonTheme.size.iconTiny), tint = colors.warning.toColor())
            AgentToolStatus.Completed, is AgentToolStatus.Other -> Unit
        }
    }
}

private fun kindIcon(kind: AgentToolKind): ImageVector = when (kind) {
    AgentToolKind.Read -> WontonIcon.Document
    AgentToolKind.Edit -> WontonIcon.Compose
    AgentToolKind.Delete -> WontonIcon.Trash
    AgentToolKind.Move -> WontonIcon.Folder
    AgentToolKind.Search -> WontonIcon.Search
    AgentToolKind.Execute -> WontonIcon.Code
    AgentToolKind.Think -> WontonIcon.Agent
    AgentToolKind.Fetch -> WontonIcon.Globe
    is AgentToolKind.Other -> WontonIcon.Wrench
}
