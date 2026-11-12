package app.wonton.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
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

/** 聊天页底部的输入栏。 */
@Composable
fun Composer(
    roomId: String,
    draft: String,
    onDraftChange: (String) -> Unit,
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
    val pickPhoto = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
        if (uri != null) {
            scope.launch {
                PhotoUpload.make(context, uri).fold(
                    onSuccess = { upload -> onAction(Action.SendImage(roomId, upload)) },
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
    OnboardingChatPrompts(roomId)
    Row(Modifier.fillMaxWidth().padding(spacing.s4), verticalAlignment = Alignment.Bottom) {
        WontonIconButton(WontonIcon.Attach, stringResource(R.string.composer_attach), { isChoosingAttachment = true })
        ComposerField(
            value = draft,
            onValueChange = onDraftChange,
            modifier = Modifier.weight(1f)
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
                onAction(Action.SendText(roomId, draft))
                onDraftChange("")
            }
        }
    }
    if (isChoosingAttachment) {
        WontonActionSheet(
            actions = listOf(
                WontonSheetAction(stringResource(R.string.composer_photo), WontonIcon.Image) {
                    pickPhoto.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly))
                },
                WontonSheetAction(stringResource(R.string.composer_file), WontonIcon.Attach) {
                    pickFiles.launch(arrayOf("*/*"))
                },
            ),
            onDismiss = { isChoosingAttachment = false },
        )
    }
}

@Composable
private fun ComposerField(value: String, onValueChange: (String) -> Unit, modifier: Modifier) {
    val colors = WontonTheme.colors
    val typography = WontonTheme.typography
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        maxLines = MAX_LINES,
        textStyle = typography.body.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        modifier = modifier,
        decorationBox = { field ->
            Box(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.composerField)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.composerField))
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (value.isEmpty()) {
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
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.onPrimary.toColor(),
            )
        }
    }
}
