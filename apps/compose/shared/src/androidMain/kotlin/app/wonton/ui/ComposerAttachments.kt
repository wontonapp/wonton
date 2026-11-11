package app.wonton.ui

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.ReplyQuote
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

internal class ComposerAttachments(
    val pasteListener: androidx.compose.foundation.content.ReceiveContentListener,
    val pendingPaste: Uri?,
    val pastedImage: Boolean,
    val choosing: Boolean,
    val showChooser: () -> Unit,
    val dismissChooser: () -> Unit,
    val dismissPaste: () -> Unit,
    val pickPhoto: () -> Unit,
    val pickFiles: () -> Unit,
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun rememberComposerAttachments(
    roomId: String,
    reply: ReplyQuote?,
    onAction: (Action) -> Unit,
    onReplySent: () -> Unit,
    onReject: (Problem) -> Unit,
): ComposerAttachments {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var choosing by rememberSaveable { mutableStateOf(false) }
    var pending by rememberSaveable(roomId) { mutableStateOf<Uri?>(null) }
    var image by rememberSaveable(roomId) { mutableStateOf(false) }
    val listener = remember(roomId) {
        pasteContentListener(
            context.applicationContext.contentResolver,
            onImage = { image = true; pending = it },
            onFile = { image = false; pending = it },
        )
    }
    val pickPhoto = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
        if (uri != null) scope.launch { sendPhoto(context, roomId, reply, uri, onAction, onReplySent, onReject) }
    }
    val pickFiles = rememberLauncherForActivityResult(OpenMultipleDocuments()) { uris ->
        uris.forEach { onAction(uploadAction(context, it, roomId)) }
    }
    return ComposerAttachments(
        listener,
        pending,
        image,
        choosing,
        showChooser = { choosing = true },
        dismissChooser = { choosing = false },
        dismissPaste = { pending = null },
        pickPhoto = {
            choosing = false
            pickPhoto.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly))
        },
        pickFiles = {
            choosing = false
            pickFiles.launch(arrayOf("*/*"))
        },
    )
}

private suspend fun sendPhoto(
    context: Context,
    roomId: String,
    reply: ReplyQuote?,
    uri: Uri,
    onAction: (Action) -> Unit,
    onReplySent: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    val unreadable = context.getString(R.string.photo_unreadable)
    val tryAnother = context.getString(R.string.photo_try_another)
    PhotoUpload.make(context, uri).fold(
        onSuccess = { upload ->
            onAction(Action.SendImage(roomId, upload, reply?.eventId))
            if (reply != null) onReplySent()
        },
        onFailure = { failure -> onReject(Problem(unreadable, tryAnother, failure.message)) },
    )
}

@Composable
internal fun ComposerAttachmentChooser(
    state: ComposerAttachments,
) {
    if (!state.choosing) return
    WontonActionSheet(
        actions = listOf(
            WontonSheetAction(
                stringResource(R.string.composer_photo), WontonIcon.Image, onClick = state.pickPhoto,
            ),
            WontonSheetAction(
                stringResource(R.string.composer_file), WontonIcon.Attach, onClick = state.pickFiles,
            ),
        ),
        onDismiss = state.dismissChooser,
    )
}
