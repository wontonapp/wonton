package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.Transfer
import app.wonton.protocol.TransferAction
import app.wonton.protocol.TransferDirection
import app.wonton.protocol.TransferState

/** 输入框上方进行中和要处理的传输，还有下载完的入口。上传完的已经是房间里的一条消息。 */
@Composable
fun TransfersStrip(roomId: String, core: ClientCore) {
    val view = LocalFiles.current.transfers
    val listed = remember(view, roomId) { core.roomTransfers(roomId) }
    var showingDownloaded by remember { mutableStateOf(false) }
    if (view.downloaded.isEmpty()) showingDownloaded = false
    if (listed.here.isEmpty() && listed.elsewhere.isEmpty() && view.downloaded.isEmpty()) return
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        listed.here.forEach { TransferRow(it) }
        listed.elsewhere.forEach { TransferRow(it.transfer, Destination(it.roomName)) }
        if (view.downloaded.isNotEmpty()) {
            TextButton(onClick = { showingDownloaded = true }, Modifier.align(Alignment.End)) {
                Text("已下载 ${view.downloaded.size} 个", style = MaterialTheme.typography.labelMedium)
            }
        }
    }
    if (showingDownloaded && view.downloaded.isNotEmpty()) {
        AlertDialog(
            onDismissRequest = { showingDownloaded = false },
            title = { Text("已下载") },
            text = { LazyColumn { items(view.downloaded, key = { it.id }) { TransferRow(it) } } },
            confirmButton = { TextButton(onClick = { showingDownloaded = false }) { Text("完成") } },
        )
    }
}

/** 别的房间的传输要写明发到哪。 */
private class Destination(val roomName: String?)

@Composable
private fun TransferRow(transfer: Transfer, destination: Destination? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(if (transfer.direction == TransferDirection.UPLOAD) "↑" else "↓")
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(transfer.name, style = MaterialTheme.typography.labelMedium, maxLines = 1)
                destination?.let { DestinationText(it) }
                TransferDetail(transfer)
            }
        }
        Row(Modifier.align(Alignment.End)) {
            transfer.actions.forEach { TransferButton(transfer, it) }
        }
    }
}

@Composable
private fun DestinationText(destination: Destination) {
    val small = MaterialTheme.typography.labelSmall
    val room = destination.roomName
    if (room != null) {
        Text("发到 $room", style = small)
    } else {
        Text("目标房间不在这台服务器的列表里", style = small, color = MaterialTheme.colorScheme.error)
    }
}

@Composable
private fun TransferDetail(transfer: Transfer) {
    val small = MaterialTheme.typography.labelSmall
    val error = MaterialTheme.colorScheme.error
    when (val state = transfer.state) {
        TransferState.Running -> TransferBar(transfer)
        TransferState.Paused -> {
            val done = sizeText(transfer.transferredBytes)
            val because = transfer.pausedBecause
            if (because != null) {
                Text("暂停，已传 $done：$because", style = small, color = error)
            } else {
                Text("等连接恢复后接着传，已传 $done", style = small)
            }
        }
        is TransferState.LinkNotSent -> Text("已传完，链接没发出：${state.message}", style = small, color = error)
        is TransferState.Failed -> Text(state.message, style = small, color = error)
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
    val (label, onClick) = when (action) {
        TransferAction.CANCEL -> "取消" to { files.onAction(Action.CancelTransfer(transfer.id)) }
        TransferAction.DISMISS -> "清除" to { files.onAction(Action.DismissTransfer(transfer.id, false)) }
        TransferAction.DISMISS_DOWNLOADED -> "清理" to { confirming = true }
        TransferAction.RESEND_LINK -> "重发链接" to { files.onAction(Action.ResendFileLink(transfer.id)) }
        TransferAction.STOP_SENDING -> "不发了" to { files.onAction(Action.CancelTransfer(transfer.id)) }
        TransferAction.OPEN -> "打开" to {
            path?.let { openWithSystem(context, it, null)?.let(files.onReject) } ?: Unit
        }
        TransferAction.SHARE -> "分享" to { path?.let { shareWithSystem(context, it) } ?: Unit }
    }
    TextButton(onClick = onClick) { Text(label, style = MaterialTheme.typography.labelMedium) }
    if (confirming) DismissDownloadedDialog(transfer, onClose = { confirming = false })
}

@Composable
private fun DismissDownloadedDialog(transfer: Transfer, onClose: () -> Unit) {
    val files = LocalFiles.current
    val dismiss = { deleteLocalFile: Boolean ->
        onClose()
        files.onAction(Action.DismissTransfer(transfer.id, deleteLocalFile))
    }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("清理 ${transfer.name}") },
        text = {
            Column {
                TextButton(onClick = { dismiss(false) }) { Text("只清记录") }
                TextButton(onClick = { dismiss(true) }) {
                    Text("连手机上的文件一起删", color = MaterialTheme.colorScheme.error)
                }
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text("取消") } },
    )
}
