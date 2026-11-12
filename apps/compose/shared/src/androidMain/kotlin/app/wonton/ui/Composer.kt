package app.wonton.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments
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
import kotlinx.coroutines.launch

private const val MAX_LINES = 6

/**
 * 聊天页底部的输入栏。`reply` 是正在回复的消息，画在输入框上方；`focusToken` 变一次就把光标放回输入框（开始回复时）。
 * 发送时把回复对象的事件 ID 一起交给核心。
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Composer(
    roomId: String,
    draft: String,
    reply: ReplyQuote?,
    focusToken: Int,
    core: ClientCore,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReplySent: () -> Unit,
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
    var isChoosingAttachment by rememberSaveable { mutableStateOf(false) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // 字符串资源在协程里取不到，弹窗前先取好。
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    var pendingPaste by rememberSaveable(roomId) { mutableStateOf<Uri?>(null) }
    var pastedImage by rememberSaveable(roomId) { mutableStateOf(false) }
    val pasteListener = remember(roomId) {
        pasteContentListener(
            context.applicationContext.contentResolver,
            onImage = { pastedImage = true; pendingPaste = it },
            onFile = { pastedImage = false; pendingPaste = it },
        )
    }
    val pickPhoto = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
        if (uri != null) {
            scope.launch {
                PhotoUpload.make(context, uri).fold(
                    onSuccess = { upload ->
                        onAction(sendImageAction(roomId, upload, reply))
                        if (reply != null) onReplySent()
                    },
                    onFailure = { failure -> onReject(Problem(photoUnreadable, photoTryAnother, failure.message)) },
                )
            }
        }
    }
    val pickFiles = rememberLauncherForActivityResult(OpenMultipleDocuments()) { uris ->
        uris.forEach { uri -> onAction(uploadAction(context, uri, roomId)) }
    }
    val spacing = WontonTheme.spacing
    val size = WontonTheme.size
    val isEmpty = draft.isBlank()
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(focusToken) { if (focusToken > 0) focusRequester.requestFocus() }
    OnboardingChatPrompts(roomId)
    if (reply != null) ComposerReply(reply, core, onReplyCancel)
    Row(Modifier.fillMaxWidth().padding(spacing.s4), verticalAlignment = Alignment.Bottom) {
        WontonIconButton(WontonIcon.Attach, stringResource(R.string.composer_attach), { isChoosingAttachment = true })
        ComposerField(
            draft,
            onDraftChange,
            pasteListener,
            onHardwareSend = { text ->
                onAction(sendTextAction(roomId, text, reply))
                onSent()
            },
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
            SendButton {
                onAction(sendTextAction(roomId, draft, reply))
                onSent()
            }
        }
    }
    ComposerAttachmentContent(
        choosing = isChoosingAttachment,
        onDismissChooser = { isChoosingAttachment = false },
        onPickPhoto = { pickPhoto.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly)) },
        onPickFiles = { pickFiles.launch(arrayOf("*/*")) },
        pendingPaste = pendingPaste,
        pastedImage = pastedImage,
        roomId = roomId,
        reply = reply,
        onAction = onAction,
        onReplySent = onReplySent,
        onDismissPaste = { pendingPaste = null },
        onReject = onReject,
    )

}

@Composable
private fun ComposerAttachmentContent(
    choosing: Boolean,
    onDismissChooser: () -> Unit,
    onPickPhoto: () -> Unit,
    onPickFiles: () -> Unit,
    pendingPaste: Uri?,
    pastedImage: Boolean,
    roomId: String,
    reply: ReplyQuote?,
    onAction: (Action) -> Unit,
    onReplySent: () -> Unit,
    onDismissPaste: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    if (choosing) {
        WontonActionSheet(
            actions = listOf(
                WontonSheetAction(
                    stringResource(R.string.composer_photo), WontonIcon.Image, onClick = onPickPhoto,
                ),
                WontonSheetAction(
                    stringResource(R.string.composer_file), WontonIcon.Attach, onClick = onPickFiles,
                ),
            ),
            onDismiss = onDismissChooser,
        )
    }
    pendingPaste?.let { uri ->
        if (pastedImage) {
            PastePreviewSheet(
                uri,
                onSend = { upload ->
                    onAction(sendImageAction(roomId, upload, reply))
                    if (reply != null) onReplySent()
                },
                onDismiss = onDismissPaste,
                onReject = onReject,
            )
        } else {
            FilePastePreviewSheet(
                uri,
                onSend = { action -> onAction(action) },
                roomId = roomId,
                onDismiss = onDismissPaste,
                onReject = onReject,
            )
        }
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

/** 发图片的一条时间线操作；两个调用点（选图、粘贴）共用，也把行收短。 */
private fun sendTextAction(roomId: String, text: String, reply: ReplyQuote?): Action =
    Action.Timeline(TimelineAction.SendText(TimelineTarget(roomId, null), text, reply?.eventId))

private fun sendImageAction(roomId: String, upload: ImageUpload, reply: ReplyQuote?): Action =
    Action.Timeline(TimelineAction.SendImage(TimelineTarget(roomId, null), upload, reply?.eventId))
