package app.wonton.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments
import androidx.activity.result.contract.ActivityResultContracts.PickMultipleVisualMedia
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
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
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ComposerViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageUpload
import app.wonton.protocol.ReplyQuote
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Attach
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Send
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val MAX_LINES = 6

/**
 * 聊天页底部的输入栏。`reply` 是正在回复的消息，画在输入框上方；`focusToken` 变一次就把光标放回输入框（开始回复时）。
 * 相册、文件和粘贴进来的东西都先进附件区（ADR-0094，`composer` 是核心给的附件区和待发消息），只有发送按钮把字和附件
 * 一起发出去，回复对象的事件 ID 一起交给核心。
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Composer(
    roomId: String,
    draft: String,
    reply: ReplyQuote?,
    focusToken: Int,
    composer: ComposerViewModel?,
    core: ClientCore,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReplyCancel: () -> Unit,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
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
    val attach = rememberAttachments(roomId, composer, core, onReject)
    val spacing = WontonTheme.spacing
    val size = WontonTheme.size
    val isEmpty = draft.isBlank() && attachments.isEmpty()
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(focusToken) { if (focusToken > 0) focusRequester.requestFocus() }
    val send = { text: String ->
        onAction(sendAction(roomId, text, reply, viaTray))
        onSent()
    }
    OnboardingChatPrompts(roomId)
    if (reply != null) ComposerReply(reply, core, onReplyCancel)
    ComposerTray(attachments, roomId, core, onAction)
    Row(Modifier.fillMaxWidth().padding(spacing.s4), verticalAlignment = Alignment.Bottom) {
        WontonIconButton(WontonIcon.Attach, stringResource(R.string.composer_attach), attach.choose)
        ComposerField(
            draft,
            onDraftChange,
            attach.pasteListener,
            onHardwareSend = send,
            modifier = Modifier.weight(1f)
                .focusRequester(focusRequester)
                .padding(vertical = (size.target - size.composerField) / 2)
                .padding(end = if (isEmpty) spacing.s12 else 0.dp),
        )
        val motion = tween<Float>(WontonTheme.motion.feedback)
        AnimatedVisibility(
            visible = !isEmpty,
            enter = scaleIn(motion) + fadeIn(motion),
            exit = scaleOut(motion) + fadeOut(motion),
        ) {
            SendButton { send(draft) }
        }
    }
    attach.chooser()
}

/** 附件区的入口：选照片、选文件和粘贴，都只放进附件区，不发。 */
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
    val addPhotos = { uris: List<Uri> ->
        scope.launch {
            // 按选的顺序放：发出去也是这个顺序。
            uris.forEach { uri ->
                PhotoUpload.make(context, uri).fold(
                    onSuccess = { upload -> addImage(core, roomId, upload)?.let(onReject) },
                    onFailure = { failure -> onReject(Problem(photoUnreadable, photoTryAnother, failure.message)) },
                )
            }
        }
        Unit
    }
    val addFiles = { uris: List<Uri> ->
        // 文件选择器不能限制选几个：超出的由核心拒收，说一声附件区满了。
        uris.forEach { uri -> core.attachFile(context, uri, roomId)?.let(onReject) }
    }
    val pickPhoto = rememberLauncherForActivityResult(PickVisualMedia()) { uri -> uri?.let { addPhotos(listOf(it)) } }
    // 系统多选的上限要大于 1，契约换了上限才重新登记。
    val multiple = remember(remaining) { PickMultipleVisualMedia(remaining.coerceAtLeast(2)) }
    val pickPhotos = rememberLauncherForActivityResult(multiple) { uris -> addPhotos(uris.take(remaining)) }
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
        choose = { choosing = true },
        pasteListener = pasteListener,
        chooser = {
            if (choosing) {
                WontonActionSheet(
                    actions = listOf(
                        WontonSheetAction(stringResource(R.string.composer_photo), WontonIcon.Image) {
                            whenRoom {
                                val request = PickVisualMediaRequest(PickVisualMedia.ImageOnly)
                                if (remaining > 1) pickPhotos.launch(request) else pickPhoto.launch(request)
                            }
                        },
                        WontonSheetAction(stringResource(R.string.composer_file), WontonIcon.Attach) {
                            whenRoom { pickFiles.launch(arrayOf("*/*")) }
                        },
                    ),
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
        lineLimits = TextFieldLineLimits.MultiLine(maxHeightInLines = MAX_LINES),
        textStyle = typography.body.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        decorator = TextFieldDecorator { field ->
            Box(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.composerField)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.composerField))
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
private fun SendButton(onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val sendLabel = stringResource(R.string.composer_send)
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
                WontonIcon.Send,
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
private fun sendAction(roomId: String, text: String, reply: ReplyQuote?, viaTray: Boolean): Action {
    val target = TimelineTarget(roomId, null)
    return Action.Timeline(
        if (viaTray) {
            TimelineAction.SendComposer(target, text, reply?.eventId)
        } else {
            TimelineAction.SendText(target, text, reply?.eventId)
        },
    )
}
