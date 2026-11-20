package app.wonton.ui

import android.content.res.Resources
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.MessageAction
import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageRun
import app.wonton.protocol.MessageStatus
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
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Checks
import app.wonton.ui.icons.Clock
import app.wonton.ui.icons.LockKey
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date

/**
 * `showsSender`：私聊里对方是谁看标题就知道，不在每条消息上重复名字；群里只在一次连发的第一条上写名字，
 * 别人那条连发的尾条气泡旁边画头像（`isGroup` 是群里，头像跟组尾走）。`joinsPrevious`、`joinsNext`：这一条在它那一组里
 * 排第几（`run`），决定相接的角、尾巴画不画。`focused`、`emphasize`：列表刚滚到这一条，`emphasize` 时背景闪一下。
 * `onRetry`、`onCancel` 处理发送失败的这一条，`onCardAction` 是确认卡片上的操作。
 * `onOpenAgentCard` 打开这张等回答的卡片，`onAnswerAgentChoice` 回答 Bot 自己的提问（ADR-0082）。
 * `onJumpQuote` 点引用跳到原消息，`onAnswerChoice` 点这条消息下面的选项（ADR-0109）。
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
    onJumpQuote: (String) -> Unit,
    onAnswerChoice: (String) -> Unit,
    focused: Boolean = false,
    emphasize: Boolean = false,
    flushesPrevious: Boolean = false,
    flushesNext: Boolean = false,
) {
    val flash = remember(item.id) { Animatable(0f) }
    LaunchedEffect(focused, emphasize) {
        if (focused && emphasize) {
            flash.snapTo(1f)
            flash.animateTo(0f, tween(WontonTheme.motion.jumpHighlight))
        }
    }
    val highlight = if (flash.value > 0f) {
        Modifier.background(
            WontonTheme.colors.accentFill.toColor().copy(alpha = FLASH_ALPHA * flash.value),
            RoundedCornerShape(WontonTheme.radii.control),
        )
    } else {
        Modifier
    }
    Box(Modifier.fillMaxWidth().then(highlight).testTag("item-${item.id}")) {
        when (val content = item.content) {
            is TimelineContent.Message -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                MessageBubble(
                    content.message,
                    core,
                    showsSender,
                    showsSender && !joinsPrevious,
                    joinsPrevious,
                    joinsNext,
                    onRetry,
                    onCancel,
                    onCardAction,
                    { message, bounds -> onOpenMenu(item, message, bounds) },
                    onReply,
                    onOpenSideChat,
                    onJumpQuote,
                    onAnswerChoice,
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

@Composable
private fun MessageBubble(
    message: TimelineMessage,
    core: ClientCore,
    isGroup: Boolean,
    showsSender: Boolean,
    joinsPrevious: Boolean,
    joinsNext: Boolean,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
    onCardAction: (Action) -> Unit,
    onOpenMenu: (TimelineMessage, Rect) -> Unit,
    onReply: (TimelineMessage) -> Unit,
    onOpenSideChat: (String) -> Unit,
    onJumpQuote: (String) -> Unit,
    onAnswerChoice: (String) -> Unit,
    flushesPrevious: Boolean = false,
    flushesNext: Boolean = false,
) {
    val colors = WontonTheme.colors
    val isOwn = message.isOwn
    val gutter = WontonTheme.spacing.s48
    var bounds by remember { mutableStateOf(Rect.Zero) }
    val haptics = LocalHapticFeedback.current
    val showsMenu = message.actions.isNotEmpty()
    val swipe = rememberSwipeToReply(message.replyAction != null) { onReply(message) }
    val openMenu = {
        haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
        onOpenMenu(message, bounds)
    }
    val time = messageTime(message)
    // 点一下开菜单；菜单里没有这一条的操作时什么也不做。时间一直画在气泡里，读屏照样念得到。
    val pressable = Modifier
        .semantics { stateDescription = time }
        .combinedClickable(
            interactionSource = null,
            indication = null,
            onLongClick = openMenu.takeIf { showsMenu },
            onClick = { if (showsMenu) openMenu() },
        )
    Column(
        Modifier.fillMaxWidth().padding(start = if (isOwn) gutter else 0.dp, end = if (isOwn) 0.dp else gutter),
        horizontalAlignment = if (isOwn) Alignment.End else Alignment.Start,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (showsSender && !isOwn) SenderNameLine(message.senderName)
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val bubbleWidth = maxWidth * WontonTheme.ratio.bubbleWidth
            // 右侧的箭头位置留在气泡的宽度之外，气泡本身还是最宽 80%。
            val rowWidth = bubbleWidth + WontonTheme.size.iconSmall + WontonTheme.spacing.s8
            Box(
                Modifier.fillMaxWidth().then(pressable),
                contentAlignment = if (isOwn) Alignment.CenterEnd else Alignment.CenterStart,
            ) {
                // 有选项时这一列按气泡的自然宽度和按钮行撑开，按钮跟着列宽走、不越过气泡（ADR-0109 第 7 节）。
                Column(columnWrap(message.choices != null).widthIn(max = rowWidth)) {
                    // 群聊里别人的气泡左边先留出头像那一格，没有组尾头像的照样留，一组里的气泡起点才齐。
                    val avatarColumn = isGroup && !isOwn
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.Bottom,
                        horizontalArrangement = rowArrangement(isOwn),
                    ) {
                        if (avatarColumn) SenderAvatarCell(message, core, joinsNext)
                        val bubble: @Composable () -> Unit = {
                            BubbleSurface(
                                message, core, joinsPrevious, joinsNext,
                                Modifier.fillMaxWidth()
                                    .then(swipe.modifier)
                                    .onGloballyPositioned { bounds = it.boundsInRoot() },
                                onCardAction,
                                onLongPress = openMenu.takeIf { showsMenu },
                                onJumpQuote = onJumpQuote,
                                flushesPrevious = flushesPrevious,
                                flushesNext = flushesNext,
                            )
                        }
                        (message.sendState as? SendState.Failed)?.let { failed ->
                            FailedMark(failed, onRetry, onCancel)
                        }
                        Box(bubbleWrap(message.choices != null, bubbleWidth).testTag("bubble")) { bubble() }
                        SwipeHint(swipe)
                    }
                    message.choices?.let { choices ->
                        MessageChoicesRow(choices) { key -> onAnswerChoice(key) }
                    }
                }
            }
        }
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
    onCardAction: (Action) -> Unit,
    onLongPress: (() -> Unit)? = null,
    onJumpQuote: (String) -> Unit = {},
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
    // 尾巴占掉的那一边让出宽度，文字不压在尾巴上。
    val tail = (if (joinsNext) 0.dp else WontonTheme.size.bubbleTail)
    val reserve = footerReserve(message, LocalContext.current.resources)
    // 同一次发送里贴着的照片不留上下衬垫，贴成一摞（ADR-0094）。脚注压着的最后一行自带行高，底下少留一点，
    // 脚注才整个落在那行里、不会顶到上一行字的下缘。
    val bottom = when {
        flushesNext -> 0.dp
        reserve != null -> WontonTheme.spacing.s4
        else -> WontonTheme.spacing.s8
    }
    val padded = modifier.then(fill)
        .padding(start = if (isOwn) 0.dp else tail)
        .padding(end = if (isOwn) tail else 0.dp)
        .padding(horizontal = WontonTheme.spacing.s12)
        .padding(top = if (flushesPrevious) 0.dp else WontonTheme.spacing.s8)
        .padding(bottom = bottom)
    val content = if (isOwn) colors.textOnAccentFill else colors.textPrimary
    // 纯图片没有文字行，时间压在图上的胶囊里；有说明文字的图片把它放进最后一行说明。
    val overImage = message.body is MessageContent.Image && (message.body as MessageContent.Image).image.caption == null
    val styledBody: @Composable () -> Unit = {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            message.reply?.let {
                QuotedMessageRow(it, core, Modifier.testTag("quote"), onTint = content.toColor(), onClick = onJumpQuote)
            }
            CompositionLocalProvider(
                LocalContentColor provides content.toColor(),
                LocalTextStyle provides WontonTheme.typography.body,
            ) {
                MessageBody(message.body, core, message.sendState, reserve, onCardAction, onLongPress)
            }
        }
    }
    if (reserve != null) {
        // 正文末尾有一段和脚注等宽的透明占位，脚注压在它上面。放不下时占位自己换行，脚注就落在新的一行。
        Box(padded) {
            styledBody()
            // 占位文字已经念过一遍了。
            MessageFooter(message, Modifier.align(Alignment.BottomEnd).clearAndSetSemantics {})
        }
    } else if (overImage) {
        Box(padded) {
            styledBody()
            TimeCapsule(message, Modifier.align(Alignment.BottomEnd))
        }
    } else {
        Column(padded, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            styledBody()
            MessageFooter(message, Modifier.align(Alignment.End))
        }
    }
}

private fun messageTime(message: TimelineMessage): String =
    DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(message.timestampMs.toLong()))

/**
 * 脚注能压在最后一行上时，给它留的透明文字，宽度要和真实脚注一样：编辑标记、时间、回执一个不少，
 * 少一段脚注就会压到正文上。文件卡片和代码块、表格以及纯图片后面没有文字行，脚注单独占一行。
 * 开头是普通空格：不能和前一个词连着折行。
 */
private fun footerReserve(message: TimelineMessage, resources: Resources): String? {
    val body = when (val content = message.body) {
        is MessageContent.Text -> content.blocks
        is MessageContent.Image -> content.image.caption?.blocks
        else -> null
    }
    val inlineLast = when (body?.lastOrNull()?.kind) {
        RichBlockKind.Paragraph, RichBlockKind.Heading, is RichBlockKind.ListItem -> true
        else -> false
    }
    val files = (message.body as? MessageContent.Text)?.files
    if (!inlineLast || !files.isNullOrEmpty()) return null
    val edited = if (message.edited) resources.getString(R.string.timeline_edited) else ""
    val parts = buildList {
        if (edited.isNotEmpty()) add(edited)
        add(messageTime(message))
        // 回执图标是 14dp，一个字宽不够；不折行空格按 6 个留，字体放大缩小时两边等比。
        if (message.isOwn && (message.status == MessageStatus.SENT || message.status == MessageStatus.READ)) {
            add("\u00A0".repeat(6))
        }
    }
    // 开头留一个普通空格：不能和前一个词连着折行；中间的空格换成不折行，脚注才不会被拆开。
    return " " + parts.joinToString("  ").replace(' ', '\u00A0')
}

/**
 * 气泡右下角的脚注，和正文同一个色系：编辑过、时间、回执。自己的回执是一个钟表（服务器收到了）、一个勾（对方收到了）
 * 或两个勾（对方读了）；别人的只写时间。
 */
@Composable
private fun MessageFooter(message: TimelineMessage, modifier: Modifier) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.caption
    val color = (if (message.isOwn) colors.accentInkOnFill else colors.textSecondary).toColor()
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        if (message.edited) Text(stringResource(R.string.timeline_edited), style = style, color = color)
        // 还在发的时间换成一个钟表，发出去以后照常写时间。
        if (message.sendState is SendState.Sending) {
            Icon(
                WontonIcon.Clock,
                // 装饰：时间在气泡的语义里。
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconTiny).alpha(FOOTER_ICON_ALPHA),
                tint = color,
            )
        } else {
            Text(
                messageTime(message),
                Modifier.testTag("messageTime"),
                style = style,
                color = color,
            )
        }
        if (message.isOwn) ReceiptMark(message.status, color)
    }
}

@Composable
private fun ReceiptMark(status: MessageStatus?, color: Color) {
    val icon = when (status) {
        MessageStatus.SENT -> WontonIcon.Check
        MessageStatus.READ -> WontonIcon.Checks
        else -> return
    }
    Icon(
        icon,
        // 装饰：电话里的回执语义由消息本身给不出，但时间已经念到了。
        contentDescription = null,
        modifier = Modifier.size(WontonTheme.size.iconTiny),
        tint = color,
    )
}

/** 纯图片右下角压在图上的时间胶囊：半透明深底、白字。 */
@Composable
private fun TimeCapsule(message: TimelineMessage, modifier: Modifier) {
    Row(
        modifier.padding(WontonTheme.size.bubbleTail)
            .background(TIME_CAPSULE_BG, RoundedCornerShape(WontonTheme.radii.pill))
            .padding(horizontal = WontonTheme.spacing.s8, vertical = WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Text(messageTime(message), style = WontonTheme.typography.caption, color = Color.White)
        if (message.isOwn) ReceiptMark(message.status, Color.White)
    }
}

/** 这一条是不是它那一组的最后一条：组尾才画尾巴。 */
internal val TimelineItem.endsRun: Boolean
    get() = message?.let { it.run == MessageRun.ALONE || it.run == MessageRun.LAST } == true

/** 气泡后面闪光的不透明度，只在跳转落到这一条时用。 */
private const val FLASH_ALPHA = 0.16f

/** 脚注里钟表的轻重：和正文的墨色有区分。 */
private const val FOOTER_ICON_ALPHA = 0.7f

/** 图片上时间胶囊的底色。 */
private val TIME_CAPSULE_BG = Color.Black.copy(alpha = 0.45f)
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
            body.image.caption?.let { caption -> TextMessageBody(caption.blocks, emptyList(), core, trailingReserve) }
        }
        is MessageContent.File -> MessageFileCard(body.file, core, onLongPress)
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
