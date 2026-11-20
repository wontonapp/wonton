package app.wonton.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldDecorator
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AgentAction
import app.wonton.protocol.ComposerAttachment
import app.wonton.protocol.ComposerViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageUpload
import app.wonton.protocol.ReplyQuote
import app.wonton.protocol.RoomAgentStop
import app.wonton.protocol.ComposerEdit
import app.wonton.protocol.RoomCommand
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Attach
import app.wonton.ui.icons.Code
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Send
import app.wonton.ui.icons.List
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Compose
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * 聊天页底部的输入栏。`reply` 是正在回复的消息，画在输入框上方；`focusToken` 变一次就把光标放回输入框（开始回复时）。
 * 相册、文件和粘贴进来的东西都先进附件区（ADR-0094，`composer` 是核心给的附件区和待发消息），只有发送按钮把字和附件
 * 一起发出去，回复对象的事件 ID 一起交给核心。
 *
 * 附件区只有主对话有（ADR-0093、ADR-0094）：[target] 是旁聊时字和照片直接发进那个旁聊，`composer` 是 `null`；
 * 文件上传只认房间，旁聊里不给选文件。
 *
 * [commands] 是各 Bot 的斜杠命令（ADR-0054）：输入“/”时按名字前缀在输入框上方列出来，点一条就把 [RoomCommand.text]
 * 原样发出去。[dropCommand] 非空时发的是它（草稿整句就是一条完整命令，正文里的空行不算），否则发草稿。
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Composer(
    target: TimelineTarget,
    draft: String,
    reply: ReplyQuote?,
    focusToken: Int,
    composer: ComposerViewModel?,
    core: ClientCore,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReplyCancel: () -> Unit,
    editing: ComposerEdit?,
    onEditCancel: () -> Unit,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    commands: List<RoomCommand> = emptyList(),
    commandsOmitted: Int = 0,
    dropCommand: String? = null,
) {
    val incoming = LocalTabDraft.current
    LaunchedEffect(incoming.text) {
        incoming.text?.let { text ->
            onDraftChange(if (draft.isBlank()) text else "$draft\n$text")
            incoming.text = null
        }
    }
    val attachments = composer?.attachments.orEmpty()
    val viaTray = attachments.isNotEmpty() || composer?.outgoing.orEmpty().isNotEmpty()
    val roomId = target.roomId
    val inSideChat = target.threadRoot != null
    // 旁聊里照片不进附件区，选好就发；输入框里的字只当第一张的说明，回复跟着这张照片发出去。
    val sendImage = sideChatSendImage(inSideChat, target, reply, onAction, onSent, onReplyCancel)
    val attach = rememberAttachments(roomId, composer, core, onReject, sendImage, draft)
    val isEmpty = draft.isBlank() && attachments.isEmpty()
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(focusToken) { if (focusToken > 0) focusRequester.requestFocus() }
    // 核心收下了才清草稿、收回复：被拒（附件没传上去、房间不能发）时字和回复都留着等主人改。
    val send = { text: String ->
        val problem = core.send(sendAction(target, text, reply, viaTray))
        if (problem == null) onSent() else onReject(problem)
    }
    val agentStop by produceState<RoomAgentStop>(RoomAgentStop.Idle, core, target) {
        core.agentStopUpdates(target).collect { value = it }
    }
    ComposerStack(
        inSideChat = inSideChat,
        roomId = roomId,
        reply = reply,
        editing = editing,
        onEditCancel = onEditCancel,
        core = core,
        onReplyCancel = onReplyCancel,
        attachments = attachments,
        onAction = onAction,
        draft = draft,
        commands = commands,
        commandsOmitted = commandsOmitted,
        target = target,
        onSent = onSent,
        onReject = onReject,
    )
    AgentStopNotice(agentStop)
    ComposerBar(
        target = target,
        draft = draft,
        commands = commands,
        agentStop = agentStop,
        isEmpty = isEmpty,
        editing = editing != null,
        focusRequester = focusRequester,
        onDraftChange = onDraftChange,
        attach = attach,
        send = send,
        dropCommand = dropCommand,
        onAction = onAction,
    )
    attach.chooser()
}

/** 草稿以“/”开头且还没打空格时按名字前缀过滤命令（ADR-0109）；打完空格就算普通消息，菜单收起。 */
internal fun commandQuery(draft: String, commands: List<RoomCommand>): List<RoomCommand> {
    val query = draft.removePrefix("/").takeIf { draft.startsWith("/") && it.none(Char::isWhitespace) }
    return commands.filter { query != null && it.name.startsWith(query, ignoreCase = true) }
}

/** 输入框上方跟着草稿变的那一摞：引导、回复、附件区和命令菜单。 */
@Composable
private fun ComposerStack(
    inSideChat: Boolean,
    roomId: String,
    reply: ReplyQuote?,
    core: ClientCore,
    onReplyCancel: () -> Unit,
    editing: ComposerEdit?,
    onEditCancel: () -> Unit,
    attachments: List<ComposerAttachment>,
    onAction: (Action) -> Unit,
    draft: String,
    commands: List<RoomCommand>,
    commandsOmitted: Int,
    target: TimelineTarget,
    onSent: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    if (!inSideChat) OnboardingChatPrompts(roomId)
    if (editing != null) {
        ComposerEditNotice(editing.summary, onEditCancel)
    } else if (reply != null) {
        ComposerReply(reply, core, onReplyCancel)
    }
    val shown = if (editing == null) commandQuery(draft, commands) else emptyList()
    ComposerTray(attachments, roomId, core, onAction)
    if (shown.isNotEmpty()) {
        // 键盘把可用的地方压小以后菜单跟着缩，别把输入框和顶栏一起挤走。
        val menuMax = (LocalConfiguration.current.screenHeightDp * TRAY_HEIGHT_SHARE).dp
        Box(Modifier.heightIn(max = menuMax)) {
            ComposerCommands(shown, commandsOmitted) { command ->
                val action = Action.Timeline(TimelineAction.SendText(target, command.text, reply?.eventId))
                val problem = core.send(action)
                if (problem == null) onSent() else onReject(problem)
            }
        }
    }
}

/** 命令菜单最多占整块屏幕的这个比例，键盘弹出后菜单会自己缩。 */
private const val TRAY_HEIGHT_SHARE = 0.2f

/** 输入框这一行：命令菜单、回形针、输入框和发送键。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ComposerBar(
    target: TimelineTarget,
    draft: String,
    commands: List<RoomCommand>,
    agentStop: RoomAgentStop,
    isEmpty: Boolean,
    editing: Boolean,
    focusRequester: FocusRequester,
    onDraftChange: (String) -> Unit,
    attach: Attachments,
    send: (String) -> Unit,
    dropCommand: String?,
    onAction: (Action) -> Unit,
) {
    val spacing = WontonTheme.spacing
    val size = WontonTheme.size
    Row(Modifier.fillMaxWidth().padding(spacing.s4), verticalAlignment = Alignment.Bottom) {
        // 命令菜单放在回形针左边（ADR-0109），没有命令的房间里不占位置。
        AnimatedVisibility(visible = commands.isNotEmpty(), enter = fadeIn(), exit = fadeOut()) {
            WontonIconButton(
                WontonIcon.List,
                stringResource(R.string.composer_commands),
                onClick = {
                    // 点菜单按钮把“/”垫在草稿前面，再点一下草稿正好是“/”就收回。
                    onDraftChange(if (draft == "/") "" else "/$draft")
                    focusRequester.requestFocus()
                },
            )
        }
        // 编辑只改文字，附件区不动（ADR-0109 第 8 节）。
        WontonIconButton(
            WontonIcon.Attach,
            stringResource(R.string.composer_attach),
            attach.choose,
            enabled = !editing,
        )
        ComposerField(
            draft,
            onDraftChange,
            attach.pasteListener,
            onHardwareSend = { text -> send(dropCommand ?: text) },
            modifier = Modifier.weight(1f)
                .focusRequester(focusRequester)
                .padding(vertical = (size.target - size.composerField) / 2)
                .padding(end = if (isEmpty && !agentStop.showsButton) spacing.s12 else 0.dp),
        )
        // Bot 在跑时停止钮在发送钮的位置；已经打了字就两个都在，接着发和停下都够得着。
        if (agentStop.showsButton) {
            AgentStopButton(agentStop) { onAction(Action.Agent(AgentAction.StopTurns(target))) }
        }
        val motion = tween<Float>(WontonTheme.motion.feedback)
        AnimatedVisibility(
            visible = !isEmpty,
            enter = scaleIn(motion) + fadeIn(motion),
            exit = scaleOut(motion) + fadeOut(motion),
        ) {
            SendButton(editing) { send(dropCommand ?: draft) }
        }
    }
}

/** 编辑时确认发到原消息；平时按原来的发送（附件区、回复一起走）。 */
private fun submit(
    editing: ComposerEdit?,
    target: TimelineTarget,
    text: String,
    reply: ReplyQuote?,
    viaTray: Boolean,
    core: ClientCore,
    onAction: (Action) -> Unit,
    onSent: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    if (editing != null) {
        // 核心成功后自己把原草稿和回复对象放回去，编辑条跟着从更新里消失。
        onAction(Action.Timeline(TimelineAction.EditMessage(target, editing.eventId, text)))
    } else {
        val problem = core.send(sendAction(target, text, reply, viaTray))
        if (problem == null) onSent() else onReject(problem)
    }
}

/** 附件区的入口：选照片、选文件和粘贴，都只放进附件区，不发（旁聊里照片直接发，见 [Composer]）。 */
@OptIn(ExperimentalFoundationApi::class)
private class Attachments(
    val choose: () -> Unit,
    val pasteListener: ReceiveContentListener,
    val chooser: @Composable () -> Unit,
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun rememberAttachments(
    roomId: String,
    composer: ComposerViewModel?,
    core: ClientCore,
    onReject: (Problem) -> Unit,
    sendImage: ((ImageUpload, caption: String?) -> Unit)?,
    draft: String,
): Attachments {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // 字符串资源在协程里取不到，弹窗前先取好。
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    val max = composer?.maxAttachments?.toInt() ?: 0
    val trayFull = stringResource(R.string.composer_tray_full, max)
    val remaining = (max - composer?.attachments.orEmpty().size).coerceAtLeast(0)
    var choosing by rememberSaveable { mutableStateOf(false) }
    // 粘贴的回调只登记一次：回复对象换了也要发给现在的那条。
    val currentSendImage by rememberUpdatedState(sendImage)
    val addPhotos = { uris: List<Uri> ->
        scope.launch {
            // 输入框里的字只当第一张发出去的照片的说明（跟 Telegram 一样）。
            var caption = draft.trim().takeIf { it.isNotEmpty() }
            // 按选的顺序放：发出去也是这个顺序。
            uris.forEach { uri ->
                PhotoUpload.make(context, uri).fold(
                    onSuccess = { upload ->
                        val send = currentSendImage
                        if (send != null) {
                            send(upload, caption)
                            caption = null
                        } else {
                            addImage(core, roomId, upload)?.let(onReject)
                        }
                    },
                    onFailure = { failure -> onReject(Problem(photoUnreadable, photoTryAnother, failure.message)) },
                )
            }
        }
        Unit
    }
    val noFiles = Problem(titleRes = R.string.side_chat_no_files)
    val addFiles = { uris: List<Uri> ->
        if (currentSendImage != null) {
            onReject(noFiles)
        } else {
            // 文件选择器不能限制选几个：超出的由核心拒收，说一声附件区满了。
            uris.forEach { uri -> core.attachFile(context, uri, roomId)?.let(onReject) }
        }
    }
    val pickFiles = rememberLauncherForActivityResult(OpenMultipleDocuments(), addFiles)
    val pasteListener = remember(roomId) {
        pasteContentListener(
            context.applicationContext.contentResolver,
            onImage = { uri -> addPhotos(listOf(uri)) },
            onFile = { uri -> addFiles(listOf(uri)) },
        )
    }
    // 附件区还没读到时照常打开，放不进去的原因由核心给。
    val whenRoom = { open: () -> Unit ->
        if (composer != null && remaining == 0) onReject(Problem(trayFull)) else open()
    }
    return Attachments(
        choose = { whenRoom { choosing = true } },
        pasteListener = pasteListener,
        chooser = {
            if (choosing) {
                AttachmentSheet(
                    remaining = remaining,
                    allowFiles = sendImage == null,
                    onPhotos = { uris -> whenRoom { addPhotos(uris) } },
                    onFiles = { whenRoom { pickFiles.launch(arrayOf("*/*")) } },
                    onDismiss = { choosing = false },
                )
            }
        },
    )
}

/** 照片转码好了放进附件区。核心要把字节写进本机的附件区目录，放到后台。 */
private suspend fun addImage(core: ClientCore, roomId: String, upload: ImageUpload): Problem? =
    withContext(Dispatchers.IO) {
        try {
            core.addImageAttachment(TimelineTarget(roomId, null), upload)
            null
        } catch (rejection: CoreException) {
            rejection.problem
        }
    }

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ComposerField(
    draft: String,
    onDraftChange: (String) -> Unit,
    pasteListener: ReceiveContentListener,
    onHardwareSend: (String) -> Unit,
    modifier: Modifier,
) {
    val colors = WontonTheme.colors
    val typography = WontonTheme.typography
    val fieldState = remember { TextFieldState(draft) }
    // 换房间是同一个调用点重组：推上去的回调要拿到当前值，不能冻在第一次组合。
    val currentDraft by rememberUpdatedState(draft)
    val currentOnDraftChange by rememberUpdatedState(onDraftChange)
    // 草稿在别处变了（进房间读回存的草稿、桥接来的文本）时落进输入框，光标放在末尾；主人打字经 state 推上去，
    // 内容相同时不再落，避免光标跳动。
    LaunchedEffect(draft) {
        if (fieldState.text.toString() != draft) {
            fieldState.edit {
                replace(0, length, draft)
                if (draft.isEmpty()) placeCursorBeforeCharAt(0) else placeCursorAfterCharAt(draft.length - 1)
            }
        }
    }
    LaunchedEffect(fieldState) {
        snapshotFlow { fieldState.text.toString() }.collect { text ->
            if (text != currentDraft) currentOnDraftChange(text)
        }
    }
    BasicTextField(
        state = fieldState,
        modifier = modifier.contentReceiver(pasteListener).hardwareMessageKeys(fieldState, onHardwareSend),
        lineLimits = TextFieldLineLimits.MultiLine(
            maxHeightInLines = WontonTheme.ratio.composerLines.toInt(),
        ),
        textStyle = typography.body.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        decorator = TextFieldDecorator { field ->
            Box(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.composerField)
                    .background(
                        colors.surfaceSubtle.toColor().copy(alpha = 0.72f),
                        RoundedCornerShape(WontonTheme.radii.composerField),
                    )
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (fieldState.text.isEmpty()) {
                    Text(
                        stringResource(R.string.composer_message),
                        style = typography.body,
                        color = colors.textSecondary.toColor(),
                    )
                }
                field()
            }
        },
    )
}

@Composable
private fun SendButton(editing: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val sendLabel = stringResource(if (editing) R.string.composer_edit_send else R.string.composer_send)
    Box(
        Modifier.size(WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = sendLabel },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(WontonTheme.size.sendButton).background(colors.primary.toColor(), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (editing) WontonIcon.Check else WontonIcon.Send,
                // 装饰：外层的可点区域已经带了“发送”。
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.onPrimary.toColor(),
            )
        }
    }
}

/**
 * 发送按钮。附件区空着、前面也没有待发消息时照旧直接发字：不进待发消息，不闪一下；否则作为一次发送交给附件区，
 * 字也排在前面的待发消息后面，不抢到前面去。
 */
private fun sendAction(target: TimelineTarget, text: String, reply: ReplyQuote?, viaTray: Boolean): Action =
    Action.Timeline(
        if (viaTray) {
            TimelineAction.SendComposer(target, text, reply?.eventId)
        } else {
            TimelineAction.SendText(target, text, reply?.eventId)
        },
    )

/// 旁聊里的照片选好就发，输入框里的字只当说明；主对话返回 null，由附件区接管。
private fun sideChatSendImage(
    inSideChat: Boolean,
    target: TimelineTarget,
    reply: ReplyQuote?,
    onAction: (Action) -> Unit,
    onSent: () -> Unit,
    onReplyCancel: () -> Unit,
): ((ImageUpload, String?) -> Unit)? {
    if (!inSideChat) return null
    return { upload, caption ->
        onAction(
            Action.Timeline(TimelineAction.SendImage(target, upload, caption = caption, replyTo = reply?.eventId)),
        )
        if (caption != null) onSent() else if (reply != null) onReplyCancel()
    }
}
