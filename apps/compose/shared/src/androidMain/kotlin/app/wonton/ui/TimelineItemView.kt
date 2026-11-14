package app.wonton.ui

import android.content.res.Resources
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.MessageAction
import app.wonton.protocol.MessageContent
import app.wonton.protocol.RichBlockKind
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.LockKey
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Reply
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WarningCircle
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date

/**
 * `showsSender`：私聊里对方是谁看标题就知道，不在每条消息上重复名字。`joinsPrevious`、`joinsNext`：和上一条、下一条是
 * 不是同一个人连着发的。`onRetry`、`onCancel` 处理发送失败的这一条，`onCardAction` 是确认卡片上的操作。
 * `onOpenAgentCard` 打开这张等回答的卡片，`onAnswerAgentChoice` 回答 Bot 自己的提问（ADR-0082）。
 * `onOpenSideChat` 进旁聊（参数是根的事件 ID），旁聊卡片和结果行都用它（ADR-0093）。
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
    onCardAction: (Action) -> Unit,
    onOpenMenu: (TimelineItem, TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    onOpenAgentCard: (String) -> Unit,
    onAnswerAgentChoice: (String) -> Unit,
    onOpenSideChat: (String) -> Unit,
    flushesPrevious: Boolean = false,
    flushesNext: Boolean = false,
) {
    when (val content = item.content) {
        // 时间不贴在气泡上：隔得久的那条上方写一行，其余的点一下气泡才露出来。
        is TimelineContent.Message -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (content.message.startsTimeGroup) CenteredNote(messageTime(content.message))
            MessageBubble(
                content.message,
                core,
                showsSender && !joinsPrevious,
                joinsPrevious,
                joinsNext,
                onRetry,
                onCancel,
                onCardAction,
                { message, bounds -> onOpenMenu(item, message, bounds) },
                onReply,
                onOpenSideChat,
                flushesPrevious,
                flushesNext,
            )
        }
        is TimelineContent.Notice -> CenteredNote(content.text)
        is TimelineContent.AgentRun -> AgentRunRowView(content, onOpenAgentCard, onAnswerAgentChoice)
        is TimelineContent.ProfileChange -> CenteredNote(
            content.previousName
                ?.let { stringResource(R.string.timeline_profile_rename, it, content.name) }
                ?: stringResource(R.string.timeline_profile_avatar, content.name),
        )
        // 分隔线带着这一天第一条消息的时间：日期和时间写在一行，那条消息上方不再另写。
        is TimelineContent.DateDivider -> CenteredNote(
            DateFormat.getDateTimeInstance(DateFormat.FULL, DateFormat.SHORT)
                .format(Date(content.timestampMs.toLong())),
        )
        TimelineContent.ReadMarker -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            NoteEdge()
            Text(
                stringResource(R.string.timeline_new_messages_below),
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.accentInk.toColor(),
            )
            NoteEdge()
        }
        TimelineContent.TimelineStart -> CenteredNote(stringResource(R.string.timeline_start))
        is TimelineContent.SideChat -> SideChatCardRow(content.card, Modifier.fillMaxWidth()) {
            onOpenSideChat(content.card.rootEventId)
        }
        is TimelineContent.SideChatResult -> SideChatResultRow(content.title, content.state) {
            onOpenSideChat(content.rootEventId)
        }
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
    onCardAction: (Action) -> Unit,
    onOpenMenu: (TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    onOpenSideChat: (String) -> Unit,
    flushesPrevious: Boolean = false,
    flushesNext: Boolean = false,
) {
    val colors = WontonTheme.colors
    val isOwn = message.isOwn
    val gutter = WontonTheme.spacing.s48
    var bounds by remember { mutableStateOf(Rect.Zero) }
    val haptics = LocalHapticFeedback.current
    val showsMenu = message.actions.isNotEmpty()
    // 点一下气泡露出这条的时间，再点收起（Material 的做法，对应 iOS 的左滑整列）。
    var showsTime by remember { mutableStateOf(false) }
    val swipe = rememberSwipeToReply(message.replyAction != null) { onReply(message) }
    // 长按开菜单；图片消息自己的点击在它上面处理，长按也从那里来（onLongPress 传下去）。
    val openMenu = {
        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
        onOpenMenu(message, bounds)
    }
    // 时间不画在气泡里了，读屏照样念得到。
    val time = messageTime(message)
    val pressable = Modifier
        .semantics { stateDescription = time }
        .combinedClickable(
            interactionSource = null,
            indication = null,
            onLongClick = openMenu.takeIf { showsMenu },
            onClick = { showsTime = !showsTime },
        )
    Column(
        Modifier.fillMaxWidth().padding(start = if (isOwn) gutter else 0.dp, end = if (isOwn) 0.dp else gutter),
        horizontalAlignment = if (isOwn) Alignment.End else Alignment.Start,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (showsSender && !isOwn) {
            Text(message.senderName, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
        }
        Box(
            Modifier.fillMaxWidth().then(pressable),
            contentAlignment = if (isOwn) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            SwipeHint(swipe)
            BubbleSurface(
                message,
                core,
                joinsPrevious,
                joinsNext,
                swipe.modifier.onGloballyPositioned { bounds = it.boundsInRoot() },
                onRetry,
                onCancel,
                onCardAction,
                onLongPress = openMenu.takeIf { showsMenu },
                flushesPrevious = flushesPrevious,
                flushesNext = flushesNext,
            )
        }
        if (showsTime) {
            Text(
                time,
                Modifier.testTag("messageTime").clearAndSetSemantics {},
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
            )
        }
        ReceiptLine(message.receipt)
        // 有回复的消息下面挂一张旁聊卡片（ADR-0093），点了进那个旁聊。
        message.sideChat?.let { card -> SideChatCardRow(card) { onOpenSideChat(card.rootEventId) } }
    }
}

/** 气泡本身。长按菜单的预览也画这个。 */
@Composable
fun BubbleSurface(
    message: TimelineMessage,
    core: ClientCore,
    joinsPrevious: Boolean,
    joinsNext: Boolean,
    modifier: Modifier,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
    onCardAction: (Action) -> Unit,
    onLongPress: (() -> Unit)? = null,
    flushesPrevious: Boolean = false,
    flushesNext: Boolean = false,
) {
    val colors = WontonTheme.colors
    val isOwn = message.isOwn
    val shape = bubbleShape(isOwn, joinsPrevious, joinsNext)
    val fill = if (isOwn) {
        Modifier.accentFill(shape)
    } else {
        Modifier.background(colors.surfaceSubtle.toColor(), shape)
    }
    // 同一次发送里贴着的照片不留上下衬垫，贴成一摞（ADR-0094）。
    val padded = modifier.then(fill)
        .padding(horizontal = WontonTheme.spacing.s12)
        .padding(top = if (flushesPrevious) 0.dp else WontonTheme.spacing.s8)
        .padding(bottom = if (flushesNext) 0.dp else WontonTheme.spacing.s8)
    val content = if (isOwn) colors.textOnAccentFill else colors.textPrimary
    val styledBody: @Composable (String?) -> Unit = { reserve ->
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            message.reply?.let { QuotedMessageRow(it, core, onTint = content.toColor()) }
            CompositionLocalProvider(
                LocalContentColor provides content.toColor(),
                LocalTextStyle provides WontonTheme.typography.body,
            ) {
                MessageBody(message.body, core, message.sendState, reserve, onCardAction, onLongPress)
            }
        }
    }
    val reserve = footerReserve(message, LocalContext.current.resources)
    if (!message.hasFooter) {
        Box(padded) { styledBody(null) }
    } else if (reserve != null) {
        // 正文末尾有一段和脚注等宽的透明占位，脚注压在它上面。放不下时占位自己换行，脚注就落在新的一行。
        Box(padded) {
            styledBody(reserve)
            // 占位文字已经念过一遍了。
            MessageFooter(
                message,
                Modifier.align(Alignment.BottomEnd).clearAndSetSemantics {},
                onRetry,
                onCancel,
            )
        }
    } else {
        Column(padded, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            styledBody(null)
            MessageFooter(message, Modifier.align(Alignment.End), onRetry, onCancel)
        }
    }
}

private fun messageTime(message: TimelineMessage): String =
    DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(message.timestampMs.toLong()))

/** 气泡里只在有话要说时才有脚注：编辑过、还在发、没发出去。时间和已读不在气泡里。 */
private val TimelineMessage.hasFooter: Boolean
    get() = edited || sendState !is SendState.Sent

/**
 * 脚注能压在最后一行上时，给它留的透明文字。发送失败的说明太长，文件卡片和代码块、表格后面也没有文字行，都单独占一行。
 * 开头是普通空格：不能和前一个词连着折行。
 */
private fun footerReserve(message: TimelineMessage, resources: Resources): String? {
    val body = message.body as? MessageContent.Text
    val inlineLast = when (body?.blocks?.lastOrNull()?.kind) {
        RichBlockKind.Paragraph, RichBlockKind.Heading, is RichBlockKind.ListItem -> true
        else -> false
    }
    if (!inlineLast || !body?.files.isNullOrEmpty() || message.sendState is SendState.Failed) return null
    val parts = buildList {
        if (message.edited) add(resources.getString(R.string.timeline_edited))
        if (message.sendState is SendState.Sending) add(resources.getString(R.string.timeline_sending))
    }
    return " \u00A0\u00A0" + parts.joinToString("  ").replace(' ', '\u00A0')
}

@Composable
private fun MessageFooter(message: TimelineMessage, modifier: Modifier, onRetry: () -> Unit, onCancel: () -> Unit) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.caption
    val color = (if (message.isOwn) colors.accentInkOnFill else colors.textSecondary).toColor()
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        if (message.edited) Text(stringResource(R.string.timeline_edited), style = style, color = color)
        when (val state = message.sendState) {
            is SendState.Sending -> Text(stringResource(R.string.timeline_sending), style = style, color = color)
            is SendState.Failed -> FailedSend(state, onRetry, onCancel)
            SendState.Sent -> Unit
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
            Modifier.clickable(role = Role.Button) { choosing = true },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Icon(
                WontonIcon.WarningCircle,
                // 装饰：失败原因写在右边的文字里。
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconTiny),
                tint = danger,
            )
            Text(
                if (state.recoverable) {
                    stringResource(R.string.timeline_send_failed_retrying)
                } else {
                    stringResource(R.string.timeline_send_failed, state.message)
                },
                style = WontonTheme.typography.caption,
                color = danger,
            )
        }
    }
    if (choosing) {
        WontonActionSheet(
            listOf(
                WontonSheetAction(stringResource(R.string.timeline_resend), WontonIcon.Refresh, onClick = onRetry),
                WontonSheetAction(
                    stringResource(R.string.common_delete), WontonIcon.Trash, isDanger = true, onClick = onCancel,
                ),
            ),
            { choosing = false },
        )
    }
}

@Composable
private fun MessageBody(
    body: MessageContent,
    core: ClientCore,
    sendState: SendState,
    trailingReserve: String?,
    onCardAction: (Action) -> Unit,
    onLongPress: (() -> Unit)? = null,
) {
    when (body) {
        is MessageContent.Text -> TextMessageBody(body.blocks, body.files, core, trailingReserve)
        is MessageContent.Image -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            MessageImage(body.image, core, sendState, onLongPress)
            // 一次发送里主人写的字是最后一张照片的说明文字（ADR-0094），画在照片下面。
            body.image.caption?.let { caption -> TextMessageBody(caption.blocks, emptyList(), core) }
        }
        is MessageContent.Confirmation -> ConfirmationCardView(body.view, onCardAction)
        MessageContent.Undecryptable -> Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            // 装饰：右边就是说明文字。
            Icon(WontonIcon.LockKey, contentDescription = null, modifier = Modifier.size(WontonTheme.size.iconSmall))
            Text(stringResource(R.string.timeline_undecryptable), style = WontonTheme.typography.supporting)
        }
        MessageContent.Redacted -> Text(
            stringResource(R.string.timeline_redacted),
            Modifier.alpha(WontonTheme.ratio.secondaryContent),
            fontStyle = FontStyle.Italic,
        )
        is MessageContent.Unsupported -> Text(
            stringResource(R.string.timeline_unsupported, body.kind),
            Modifier.alpha(WontonTheme.ratio.secondaryContent),
            style = WontonTheme.typography.supporting,
        )
    }
}

/** 这条消息的回复项，自带引用；菜单里没有就是空。 */
val TimelineMessage.replyAction: MessageAction.Reply?
    get() = actions.firstOrNull { it is MessageAction.Reply } as? MessageAction.Reply
