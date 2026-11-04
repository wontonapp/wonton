package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Answer
import app.wonton.protocol.MessageContent
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.LockKey
import app.wonton.ui.icons.WarningCircle
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date

/**
 * `showsSender`：私聊里对方是谁看标题就知道，不在每条消息上重复名字。`joinsPrevious`、`joinsNext`：和上一条、下一条是
 * 不是同一个人连着发的。`onRetry`、`onCancel` 处理发送失败的这一条，`onAnswer` 回答确认卡片。
 */
@Composable
fun TimelineItemView(
    item: TimelineItem,
    core: ClientCore,
    showsSender: Boolean,
    joinsPrevious: Boolean,
    joinsNext: Boolean,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
    onAnswer: (String, Answer) -> Unit,
) {
    when (val content = item.content) {
        is TimelineContent.Message -> MessageBubble(
            content.message,
            core,
            showsSender && !joinsPrevious,
            joinsPrevious,
            joinsNext,
            onRetry,
            onCancel,
            onAnswer,
        )
        is TimelineContent.Notice -> CenteredNote(content.text)
        is TimelineContent.DateDivider ->
            CenteredNote(DateFormat.getDateInstance(DateFormat.FULL).format(Date(content.timestampMs.toLong())))
        TimelineContent.ReadMarker -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            NoteEdge()
            Text(
                "以下是新消息",
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.accentInk.toColor(),
            )
            NoteEdge()
        }
        TimelineContent.TimelineStart -> CenteredNote("以上是聊天的开头")
    }
}

@Composable
private fun RowScope.NoteEdge() {
    Box(Modifier.weight(1f).height(WontonTheme.size.hairline).background(WontonTheme.colors.separator.toColor()))
}

@Composable
private fun CenteredNote(text: String) {
    Text(
        text,
        Modifier.fillMaxWidth(),
        style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

/** 自己的气泡靠右，相接的是右边的角；别人的相反。 */
private fun bubbleShape(isOwn: Boolean, joinsPrevious: Boolean, joinsNext: Boolean): Shape {
    val full = WontonTheme.radii.bubble
    val joined = WontonTheme.radii.bubbleJoined
    val top = if (joinsPrevious) joined else full
    val bottom = if (joinsNext) joined else full
    return RoundedCornerShape(
        topStart = if (isOwn) full else top,
        topEnd = if (isOwn) top else full,
        bottomEnd = if (isOwn) bottom else full,
        bottomStart = if (isOwn) full else bottom,
    )
}

@Composable
private fun MessageBubble(
    message: TimelineMessage,
    core: ClientCore,
    showsSender: Boolean,
    joinsPrevious: Boolean,
    joinsNext: Boolean,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
    onAnswer: (String, Answer) -> Unit,
) {
    val colors = WontonTheme.colors
    val isOwn = message.isOwn
    val gutter = WontonTheme.spacing.s48
    Column(
        Modifier.fillMaxWidth().padding(start = if (isOwn) gutter else 0.dp, end = if (isOwn) 0.dp else gutter),
        horizontalAlignment = if (isOwn) Alignment.End else Alignment.Start,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (showsSender && !isOwn) {
            Text(message.senderName, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
        }
        val shape = bubbleShape(isOwn, joinsPrevious, joinsNext)
        Column(
            Modifier.then(
                if (isOwn) {
                    Modifier.accentFill(shape)
                } else {
                    Modifier.background(colors.surfaceSubtle.toColor(), shape)
                },
            )
                .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            val content = if (isOwn) colors.textOnAccentFill else colors.textPrimary
            CompositionLocalProvider(
                LocalContentColor provides content.toColor(),
                LocalTextStyle provides WontonTheme.typography.body,
            ) {
                MessageBody(message.body, core, message.sendState, onAnswer)
            }
            MessageFooter(message, Modifier.align(Alignment.End), onRetry, onCancel)
        }
    }
}

@Composable
private fun MessageFooter(message: TimelineMessage, modifier: Modifier, onRetry: () -> Unit, onCancel: () -> Unit) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.caption
    val color = (if (message.isOwn) colors.accentInkOnFill else colors.textSecondary).toColor()
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        if (message.edited) Text("已编辑", style = style, color = color)
        Text(
            DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(message.timestampMs.toLong())),
            style = style,
            color = color,
        )
        when (val state = message.sendState) {
            is SendState.Sending -> Text("发送中", style = style, color = color)
            is SendState.Failed -> FailedSend(state, onRetry, onCancel)
            SendState.Sent -> if (message.isOwn && message.readBy.isNotEmpty()) {
                Text("已读", style = style, color = color)
            }
        }
    }
}

/** 点开选重发或删除。 */
@Composable
private fun FailedSend(state: SendState.Failed, onRetry: () -> Unit, onCancel: () -> Unit) {
    var choosing by remember { mutableStateOf(false) }
    val danger = WontonTheme.colors.danger.toColor()
    Box {
        Row(
            Modifier.clickable { choosing = true },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Icon(
                WontonIcon.WarningCircle,
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconTiny),
                tint = danger,
            )
            Text(
                if (state.recoverable) "发送失败，会自动重试" else "发送失败：${state.message}",
                style = WontonTheme.typography.caption,
                color = danger,
            )
        }
        DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
            DropdownMenuItem(
                text = { Text("重发") },
                onClick = {
                    choosing = false
                    onRetry()
                },
            )
            DropdownMenuItem(
                text = { Text("删除", color = danger) },
                onClick = {
                    choosing = false
                    onCancel()
                },
            )
        }
    }
}

@Composable
private fun MessageBody(
    body: MessageContent,
    core: ClientCore,
    sendState: SendState,
    onAnswer: (String, Answer) -> Unit,
) {
    when (body) {
        is MessageContent.Text -> TextMessageBody(body.blocks, body.files, core)
        is MessageContent.Image -> MessageImage(body.image, core, sendState)
        is MessageContent.Confirmation -> ConfirmationCardView(body.view, onAnswer)
        MessageContent.Undecryptable -> Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            Icon(WontonIcon.LockKey, contentDescription = null, modifier = Modifier.size(WontonTheme.size.iconSmall))
            Text("无法解密：本设备还没有这条消息的密钥", style = WontonTheme.typography.supporting)
        }
        MessageContent.Redacted -> Text(
            "消息已撤回",
            Modifier.alpha(WontonTheme.ratio.secondaryContent),
            fontStyle = FontStyle.Italic,
        )
        is MessageContent.Unsupported -> Text(
            "暂不支持显示的消息（${body.kind}）",
            Modifier.alpha(WontonTheme.ratio.secondaryContent),
            style = WontonTheme.typography.supporting,
        )
    }
}
