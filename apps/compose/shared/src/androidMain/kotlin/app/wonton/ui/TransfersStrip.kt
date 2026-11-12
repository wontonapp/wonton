package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextDecoration
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.Transfer
import app.wonton.protocol.TransferAction
import app.wonton.protocol.TransferDirection
import app.wonton.protocol.TransferState
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.Eye
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.Upload
import app.wonton.ui.icons.WontonIcon

/** 输入框上方进行中和要处理的传输，还有下载完的入口。上传完的已经是房间里的一条消息。 */
@Composable
fun TransfersStrip(roomId: String, core: ClientCore) {
    val view = LocalFiles.current.transfers
    val listed = remember(view, roomId) { core.roomTransfers(roomId) }
    var showingDownloaded by remember { mutableStateOf(false) }
    if (view.downloaded.isEmpty()) showingDownloaded = false
    if (listed.here.isEmpty() && listed.elsewhere.isEmpty() && view.downloaded.isEmpty()) return
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxWidth()
            .drawBehind { drawLine(colors.separator.toColor(), Offset.Zero, Offset(size.width, 0f), 1f) }
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        listed.here.forEach { TransferRow(it) }
        listed.elsewhere.forEach { TransferRow(it.transfer, Destination(it.roomName)) }
        if (view.downloaded.isNotEmpty()) {
            Text(
                pluralStringResource(R.plurals.transfer_downloaded_count, view.downloaded.size, view.downloaded.size),
                Modifier.align(Alignment.End).clickable(role = Role.Button) { showingDownloaded = true },
                style = WontonTheme.typography.caption.copy(textDecoration = TextDecoration.Underline),
                color = colors.textPrimary.toColor(),
            )
        }
    }
    if (showingDownloaded && view.downloaded.isNotEmpty()) {
        WontonSheet({ showingDownloaded = false }, stringResource(R.string.transfer_downloaded)) {
            LazyColumn(
                contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                items(view.downloaded, key = { it.id }) { TransferRow(it) }
            }
        }
    }
}

/** 别的房间的传输要写明发到哪。 */
private class Destination(val roomName: String?)

@Composable
private fun TransferRow(transfer: Transfer, destination: Destination? = null) {
    val colors = WontonTheme.colors
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(
            if (transfer.direction == TransferDirection.UPLOAD) WontonIcon.Upload else WontonIcon.Download,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                transfer.name,
                style = WontonTheme.typography.supporting,
                color = colors.textPrimary.toColor(),
                maxLines = 1,
            )
            destination?.let { DestinationText(it) }
            TransferDetail(transfer)
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            transfer.actions.forEach { TransferButton(transfer, it) }
        }
    }
}

@Composable
private fun DestinationText(destination: Destination) {
    val room = destination.roomName
    if (room != null) {
        SmallText(stringResource(R.string.transfer_sent_to, room))
    } else {
        ProblemText(stringResource(R.string.transfer_room_missing))
    }
}

@Composable
private fun SmallText(text: String) {
    Text(text, style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
}

@Composable
private fun ProblemText(text: String) {
    Text(text, style = WontonTheme.typography.caption, color = WontonTheme.colors.danger.toColor())
}

@Composable
private fun TransferDetail(transfer: Transfer) {
    val context = LocalContext.current
    when (val state = transfer.state) {
        TransferState.Running -> TransferBar(transfer)
        TransferState.Paused -> {
            val done = sizeText(context, transfer.transferredBytes)
            val because = transfer.pausedBecause
            if (because != null) {
                ProblemText(stringResource(R.string.transfer_paused_progress, done, because))
            } else {
                SmallText(stringResource(R.string.transfer_paused_waiting, done))
            }
        }
        is TransferState.LinkNotSent -> ProblemText(stringResource(R.string.transfer_link_not_sent, state.message))
        is TransferState.Failed -> ProblemText(state.message)
        TransferState.Done -> Unit
    }
}

/** 按核心给的 `actions` 画按钮。 */
@Composable
private fun TransferButton(transfer: Transfer, action: TransferAction) {
    val files = LocalFiles.current
    val context = LocalContext.current
    var confirming by remember { mutableStateOf(false) }
    val path = transfer.destination
    when (action) {
        TransferAction.CANCEL -> WontonIconButton(
            WontonIcon.Close,
            stringResource(R.string.common_cancel),
            { files.onAction(Action.CancelTransfer(transfer.id)) },
        )
        TransferAction.DISMISS -> WontonIconButton(
            WontonIcon.Close,
            stringResource(R.string.transfer_clear),
            { files.onAction(Action.DismissTransfer(transfer.id, false)) },
        )
        TransferAction.DISMISS_DOWNLOADED ->
            WontonIconButton(WontonIcon.Trash, stringResource(R.string.transfer_clean_up), { confirming = true })
        TransferAction.RESEND_LINK -> TextAction(
            stringResource(R.string.transfer_resend_link),
        ) { files.onAction(Action.ResendFileLink(transfer.id)) }
        TransferAction.STOP_SENDING -> TextAction(
            stringResource(R.string.transfer_stop_sending),
        ) { files.onAction(Action.CancelTransfer(transfer.id)) }
        TransferAction.OPEN -> WontonIconButton(WontonIcon.Eye, stringResource(R.string.transfer_open), {
            path?.let { openWithSystem(context, it, null)?.let(files.onReject) }
        })
        TransferAction.SHARE -> WontonIconButton(
            WontonIcon.Share,
            stringResource(R.string.transfer_share),
            { path?.let { shareWithSystem(context, it) } },
        )
    }
    if (confirming) DismissDownloadedDialog(transfer, onClose = { confirming = false })
}

@Composable
private fun TextAction(title: String, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s4),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            title,
            style = WontonTheme.typography.supporting.copy(textDecoration = TextDecoration.Underline),
            color = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

@Composable
private fun DismissDownloadedDialog(transfer: Transfer, onClose: () -> Unit) {
    val files = LocalFiles.current
    val dismiss = { deleteLocalFile: Boolean ->
        onClose()
        files.onAction(Action.DismissTransfer(transfer.id, deleteLocalFile))
    }
    WontonDialog(
        stringResource(R.string.transfer_cleanup_title, transfer.name),
        onDismiss = onClose,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.transfer_cleanup_record_only)) { dismiss(false) },
            WontonDialogAction(
                stringResource(R.string.transfer_cleanup_delete_file),
                WontonButtonKind.DANGER,
            ) { dismiss(true) },
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onClose),
        ),
    )
}
