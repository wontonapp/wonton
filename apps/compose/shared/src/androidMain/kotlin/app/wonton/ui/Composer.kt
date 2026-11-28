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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AgentAction
import app.wonton.protocol.ComposerAttachment
import app.wonton.protocol.ComposerEdit
import app.wonton.protocol.ComposerViewModel
import androidx.activity.compose.BackHandler
import app.wonton.protocol.ImageUpload
import app.wonton.protocol.ReplyQuote
import app.wonton.protocol.RoomAgentStop
import app.wonton.protocol.RoomCommand
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.Icon
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Attach
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Code
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Send
import app.wonton.ui.icons.List
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/**
 * 聊天页底部的输入栏。`reply` 是正在回复的消息，画在输入框上方；`focusToken` 变一次就把光标放回输入框（开始回复时）。
 * 相册、文件和粘贴进来的东西都先进附件区（ADR-0094，`composer` 是核心给的附件区和待发消息），只有发送按钮把字和附件
 * 一起发出去，回复对象的事件 ID 一起交给核心。
 *
 * 附件区只有主对话有（ADR-0093、ADR-0094）：[target] 是旁聊时字和照片直接发进那个旁聊，`composer` 是 `null`；
 * 文件上传只认房间，旁聊里不给选文件。
 *
 * [commands] 是各 Bot 的斜杠命令（ADR-0054）：输入“/”时按名字前缀在输入框上方列出来，点一条就把 [RoomCommand.text]
 * 原样发出去，发出去后草稿一个字不动、只收回复条（ADR-0109）。[dropCommand] 非空时发的是它（草稿整句就是一条完整命令，正文里的空行不算），否则发草稿。
 */
@OptIn(ExperimentalFoundationApi::class)
@Suppress("CyclomaticComplexMethod")
@Composable
fun Composer(
    target: TimelineTarget,
    draft: String,
    reply: ReplyQuote?,
    edit: ComposerEdit?,
    focusToken: Int,
    composer: ComposerViewModel?,
    core: ClientCore,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onCommandSent: () -> Unit,
    menuOpen: Boolean,
    onMenuChange: (Boolean) -> Unit,
    onReplyCancel: () -> Unit,
    onQuoteJump: (String) -> Unit,
    onEditCancel: () -> Unit,
    onRestoreDraft: (String, ReplyQuote?) -> Unit,
    onTyping: (Boolean) -> Unit,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
    commands: List<RoomCommand> = emptyList(),
    commandsOmitted: Int = 0,
    dropCommand: String? = null,
    enabled: Boolean = true,
    disabledProblem: Problem? = null,
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
    // 旁聊里的照片都不进附件区，选好就发；附言只跟着第一张（照 iOS），回复也只跟着第一张。
    // 被拒时那张就没了，原因给出去，后面的不再试；确定不能聊时先报原因，字节也不用再取。
    val sendImage: ((ImageUpload, caption: String?, replyTo: String?) -> Problem?)? = if (inSideChat) {
        { upload, caption, replyTo ->
            disabledProblem ?: core.send(
                Action.Timeline(TimelineAction.SendImage(target, upload, caption = caption, replyTo = replyTo)),
            )
        }
    } else {
        null
    }
    val attach = rememberAttachments(
        target, composer, core, draft, onDraftChange, onSent, onReject, onReplyCancel, reply, sendImage,
        disabledProblem,
    )
    // 编辑时确认的是正文：附件区有东西也不算能发。
    val isEmpty = if (edit != null) draft.isBlank() else draft.isBlank() && attachments.isEmpty()
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(focusToken) { if (focusToken > 0) focusRequester.requestFocus() }
    // 编辑时返回键先退出编辑态，不离开房间。
    BackHandler(enabled = edit != null) { onEditCancel() }
    // 核心收下了才清草稿、收回复：被拒（附件没传上去、房间不能发）时字和回复都留着等主人改。
    val send = rememberComposerSend(target, edit, reply, viaTray, core, onTyping, onSent, onReject)
    // 退出编辑态后把核心恢复的草稿读回来（编辑途中又打了字时，核心给的就是新打的）。
    RestoreDraftOnEditEnd(edit, target, core, onRestoreDraft)
    // 点完命令把焦点要回输入框：菜单项自己在点的时候会抢焦点，等这一轮重组收完（菜单收掉）再要，
    // 键盘才不会跟着菜单一起收，主人能接着打在草稿里的话。
    var refocus by remember { mutableStateOf(0) }
    LaunchedEffect(refocus) { if (refocus > 0) focusRequester.requestFocus() }
    // 刚点出去的那句还原本样留在输入框里：列表先别按“/”自己弹回来，动一个字才恢复（和 iOS 的 sentSlash 一样）。
    var sentSlash by remember { mutableStateOf<String?>(null) }
    // 草稿还是刚发出去的那一句时先压住，动了字才恢复。
    if (sentSlash != null && sentSlash != draft) sentSlash = null
    val commandSent: () -> Unit = {
        onCommandSent()
        sentSlash = draft
        onMenuChange(false)
        refocus++
    }
    val agentStop by produceState<RoomAgentStop>(RoomAgentStop.Idle, core, target) {
        core.agentStopUpdates(target).collect { value = it }
    }
    // 命令列表由“/”或菜单键打开，菜单键本身不动草稿；返回键和翻时间线先收列表（ADR-0109 第 7 节）。
    val query = commandQuery(draft, commands)
    val auto = if (sentSlash != null) emptyList() else query
    LaunchedEffect(draft) {
        onMenuChange(draft.startsWith("/") && auto.isNotEmpty())
    }
    // 草稿没以“/”开头时菜单键给整张表；以“/”开头时只给匹配的那几条；刚发出去的那句先压住。
    val shown = if (draft.startsWith("/")) auto else commands
    val listShown = menuOpen && shown.isNotEmpty()
    BackHandler(enabled = listShown) { onMenuChange(false) }
    ComposerStack(
        inSideChat = inSideChat,
        roomId = roomId,
        reply = reply,
        edit = edit,
        core = core,
        onReplyCancel = onReplyCancel,
        onEditCancel = onEditCancel,
        attachments = attachments,
        onAction = onAction,
        commands = shown,
        commandsOmitted = commandsOmitted,
        target = target,
        onCommandSent = commandSent,
        onQuoteJump = onQuoteJump,
        onReject = onReject,
        listShown = listShown,
        onPicked = { onMenuChange(false) },
        disabledProblem = disabledProblem,
    )
    AgentStopNotice(agentStop)
    ComposerBar(
        target = target,
        draft = draft,
        commands = commands,
        agentStop = agentStop,
        editing = edit != null,
        isEmpty = isEmpty,
        enabled = enabled,
        focusRequester = focusRequester,
        onDraftChange = onDraftChange,
        attach = attach,
        send = send,
        onTyping = onTyping,
        dropCommand = dropCommand,
        onAction = onAction,
        // 点菜单键时放开刚发出去那句的压制，列表按现在的草稿来。
        onMenu = { sentSlash = null; onMenuChange(!menuOpen) },
    )
    attach.chooser()
}

/** 草稿以“/”开头时按名字前缀过滤命令（ADR-0109）：名字里可以有空格，打完一条命令再打参数就不匹配了。 */
internal fun commandQuery(draft: String, commands: List<RoomCommand>): List<RoomCommand> {
    if (!draft.startsWith("/")) return emptyList()
    val query = draft.removePrefix("/")
    return commands.filter { it.name.startsWith(query, ignoreCase = true) }
}

/** 输入框上方跟着草稿变的那一摞：引导、编辑条或回复、附件区和命令菜单。 */
@Composable
private fun ComposerStack(
    inSideChat: Boolean,
    roomId: String,
    reply: ReplyQuote?,
    edit: ComposerEdit?,
    core: ClientCore,
    onReplyCancel: () -> Unit,
    onQuoteJump: (String) -> Unit,
    onEditCancel: () -> Unit,
    attachments: List<ComposerAttachment>,
    onAction: (Action) -> Unit,
    commands: List<RoomCommand>,
    commandsOmitted: Int,
    target: TimelineTarget,
    onCommandSent: () -> Unit,
    onReject: (Problem) -> Unit,
    listShown: Boolean,
    onPicked: () -> Unit,
    disabledProblem: Problem?,
) {
    if (!inSideChat) OnboardingChatPrompts(roomId)
    if (edit != null) {
        ComposerEditBar(edit, onEditCancel)
    } else if (reply != null) {
        ComposerReply(reply, core, onReplyCancel, onQuoteJump)
    }
    ComposerTray(attachments, roomId, core, onAction)
    if (listShown) {
        // 键盘把可用的地方压小以后菜单跟着缩，别把输入框和顶栏一起挤走。
        val menuMax = (LocalConfiguration.current.screenHeightDp * TRAY_HEIGHT_SHARE).dp
        Box(Modifier.heightIn(max = menuMax)) {
            ComposerCommands(commands, commandsOmitted) { command ->
                onPicked()
                if (disabledProblem != null) {
                    onReject(disabledProblem)
                } else {
                    val action = Action.Timeline(TimelineAction.SendText(target, command.text, reply?.eventId))
                    val problem = core.send(action)
                    // 点命令保留草稿：主人没打完的字还在。
                    if (problem == null) onCommandSent() else onReject(problem)
                }
            }
        }
    }
}

/** 命令菜单最多占整块屏幕的这个比例，键盘弹出后菜单会自己缩。 */
private const val TRAY_HEIGHT_SHARE = 0.2f

/** 旁聊里一次能选几张照片（和 Telegram 一摞张数一样）。 */
private const val SIDE_CHAT_PHOTOS = 10

/** 输入框这一行：命令菜单、回形针、输入框和发送键。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ComposerBar(
    target: TimelineTarget,
    draft: String,
    commands: List<RoomCommand>,
    agentStop: RoomAgentStop,
    editing: Boolean,
    isEmpty: Boolean,
    enabled: Boolean,
    focusRequester: FocusRequester,
    onDraftChange: (String) -> Unit,
    attach: Attachments,
    send: (String) -> Unit,
    onTyping: (Boolean) -> Unit,
    dropCommand: String?,
    onAction: (Action) -> Unit,
    onMenu: () -> Unit,
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
                    // 菜单键只管开合列表，草稿原样不动；键盘照样叫出来。
                    onMenu()
                    focusRequester.requestFocus()
                },
            )
        }
        WontonIconButton(WontonIcon.Attach, stringResource(R.string.composer_attach), attach.choose)
        ComposerField(
            draft,
            onDraftChange,
            attach.pasteListener,
            onHardwareSend = { text -> if (enabled) send(dropCommand ?: text) },
            enabled = enabled,
            onFocusChange = { focused -> if (!focused) onTyping(false) },
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
            SendButton(editing) { if (enabled) send(dropCommand ?: draft) }
        }
    }
}

/** 附件区的入口：选照片、选文件和粘贴，都只放进附件区，不发（旁聊里照片直接发，见 [Composer]）。 */
@OptIn(ExperimentalFoundationApi::class)
private class Attachments(
    val choose: () -> Unit,
    val pasteListener: ReceiveContentListener,
    val chooser: @Composable () -> Unit,
)

/** 旁聊和面板里照片的两种发法：粘贴一张，或面板里一次发一叠。 */
private class PhotoSenders(
    val add: (List<Uri>) -> Unit,
    val send: (List<Uri>, String) -> Unit,
)

/** 照片发送的两个入口；确定不能聊时先报同一句原因，字节也不再取。 */
@Composable
private fun rememberPhotoSenders(
    target: TimelineTarget,
    core: ClientCore,
    draft: String,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReject: (Problem) -> Unit,
    onReplyCancel: () -> Unit,
    reply: ReplyQuote?,
    sendImage: ((ImageUpload, caption: String?, replyTo: String?) -> Problem?)?,
    disabledProblem: Problem?,
    onClosed: () -> Unit,
): PhotoSenders {
    val roomId = target.roomId
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // 字符串资源在协程里取不到，弹窗前先取好。
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    // 等的功夫草稿还能改，收尾按改到哪一步来：只清掉跟着走了的那句，后来补的字留着。
    val currentDraft by rememberUpdatedState(draft)
    val currentOnDraftChange by rememberUpdatedState(onDraftChange)
    // 能不能发也按最新的算：面板开着、离线切回来时按现在的来。
    val currentDisabled by rememberUpdatedState(disabledProblem)
    // 粘贴进来的照片：旁聊里直接发，主对话里放进附件区；第一张不行就停。旁聊里的字只当第一张的说明。
    val add: (List<Uri>) -> Unit = { uris ->
        val send = sendImage
        val blocked = currentDisabled
        if (send != null && blocked != null) {
            onReject(blocked)
        } else scope.launch {
            val failure = sendPickedPhotos(
                context, uris, send, photoUnreadable, photoTryAnother, core, roomId, draft, reply?.eventId,
                onReplyCancel,
            )
            if (failure != null) {
                onReject(photoFailure(context, failure, photoTryAnother))
            } else if (send != null) {
                settleCaption(draft, currentDraft, currentOnDraftChange, onSent, onReplyCancel)
            }
        }
    }
    // 相册里选好后附言和照片一起发（照 Telegram）：附言就是输入框的草稿，面板里改的正是那一份。
    // 主对话里照片先进附件区，全部到位再发一条带附言的 SendComposer；旁聊里附言跟着第一张走（和 iOS 一样）。
    // 中途中止就停在那里、报第几张，草稿原样留着；收下了只清跟着走了的那句、收回复。
    val send: (List<Uri>, String) -> Unit = { uris, note ->
        val blocked = currentDisabled
        if (blocked != null) {
            // 确定不能聊：一句原因就收，照片的字节也不用再取。
            onReject(blocked)
            onClosed()
        } else scope.launch {
            val failure = sendPickedPhotos(
                context, uris, sendImage, photoUnreadable, photoTryAnother, core, roomId, note,
                reply?.eventId, onReplyCancel,
            )
            if (failure != null) {
                onReject(photoFailure(context, failure, photoTryAnother))
            } else if (sendImage != null) {
                settleCaption(note, currentDraft, currentOnDraftChange, onSent, onReplyCancel)
            } else {
                val problem = core.send(Action.Timeline(TimelineAction.SendComposer(target, note, reply?.eventId)))
                if (problem != null) onReject(problem) else {
                    settleCaption(note, currentDraft, currentOnDraftChange, onSent, onReplyCancel)
                }
            }
            // 先把草稿落定再收面板：面板还组合着时先落的值不会被它收尾那一写盖回去。
            onClosed()
        }
    }
    return PhotoSenders(add, send)
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun rememberAttachments(
    target: TimelineTarget,
    composer: ComposerViewModel?,
    core: ClientCore,
    draft: String,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReject: (Problem) -> Unit,
    onReplyCancel: () -> Unit,
    reply: ReplyQuote?,
    sendImage: ((ImageUpload, caption: String?, replyTo: String?) -> Problem?)?,
    disabledProblem: Problem?,
): Attachments {
    val roomId = target.roomId
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // 字符串资源在协程里取不到，弹窗前先取好。
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    val max = composer?.maxAttachments?.toInt() ?: 0
    val trayFull = stringResource(R.string.composer_tray_full, max)
    // 旁聊没有附件区，选好就发：能选多少照 Telegram 的一摞上限，不看附件区还剩几个。
    val remaining = if (sendImage != null) {
        SIDE_CHAT_PHOTOS
    } else {
        (max - composer?.attachments.orEmpty().size).coerceAtLeast(0)
    }
    var choosing by rememberSaveable { mutableStateOf(false) }
    // 粘贴的回调只登记一次：回复对象换了也要发给现在的那条。
    val currentSendImage by rememberUpdatedState(sendImage)
    // 能不能发也登记成最新的：面板开着、离线切回来时按现在的算。
    val currentDisabled by rememberUpdatedState(disabledProblem)
    val senders = rememberPhotoSenders(
        target, core, draft, onDraftChange, onSent, onReject, onReplyCancel, reply, sendImage,
        disabledProblem,
    ) { choosing = false }
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
            onImage = { uri -> senders.add(listOf(uri)) },
            onFile = { uri -> addFiles(listOf(uri)) },
        )
    }
    // 附件区还没读到时照常打开，放不进去的原因由核心给。
    val whenRoom = { open: () -> Unit ->
        if (composer != null && remaining == 0) onReject(Problem(trayFull)) else open()
    }
    return Attachments(
        choose = {
            val blocked = currentDisabled
            if (blocked != null) onReject(blocked) else whenRoom { choosing = true }
        },
        pasteListener = pasteListener,
        chooser = {
            if (choosing) {
                AttachmentSheet(
                    remaining = remaining,
                    allowFiles = sendImage == null,
                    note = draft,
                    // 面板收起以后附言框还握着整句，它迟到的回写不再盖掉刚落的草稿。
                    onNote = { text -> if (choosing) onDraftChange(text) },
                    onSend = senders.send,
                    onFiles = { whenRoom { pickFiles.launch(arrayOf("*/*")) } },
                    onDismiss = { choosing = false },
                )
            }
        },
    )
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ComposerField(
    draft: String,
    onDraftChange: (String) -> Unit,
    pasteListener: ReceiveContentListener,
    onHardwareSend: (String) -> Unit,
    onFocusChange: (Boolean) -> Unit,
    enabled: Boolean,
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
        enabled = enabled,
        modifier = modifier
            .onFocusChanged { onFocusChange(it.isFocused) }
            .contentReceiver(pasteListener)
            .hardwareMessageKeys(fieldState, onHardwareSend),
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
    // 编辑时是确认（勾），不是发送（ADR-0109 第 10 节）。
    val sendLabel = stringResource(
        if (editing) R.string.composer_edit_confirm else R.string.composer_send,
    )
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
                // 装饰：外层的可点区域已经带了“发送”（编辑时是“确认”）。
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
internal fun sendAction(target: TimelineTarget, text: String, reply: ReplyQuote?, viaTray: Boolean): Action =
    Action.Timeline(
        if (viaTray) {
            TimelineAction.SendComposer(target, text, reply?.eventId)
        } else {
            TimelineAction.SendText(target, text, reply?.eventId)
        },
    )

