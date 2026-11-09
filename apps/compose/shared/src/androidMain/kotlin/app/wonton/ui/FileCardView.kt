package app.wonton.ui

import android.content.Context
import android.graphics.BitmapFactory
import android.text.format.Formatter
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.LocalPinnableContainer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.FileCard
import app.wonton.protocol.FileCardAction
import app.wonton.protocol.FileId
import app.wonton.protocol.RichBlock
import app.wonton.protocol.FileInfo
import app.wonton.protocol.FileSource
import app.wonton.protocol.Transfer
import app.wonton.protocol.TransferState
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.File
import app.wonton.ui.icons.Play
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 文字消息：排好版的文字，下面是文件卡片。消息只有链接时没有文字。 */
@Composable
fun TextMessageBody(blocks: List<RichBlock>, files: List<FileId>, core: ClientCore, trailingReserve: String? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        if (blocks.isNotEmpty()) RichText(blocks, trailingReserve)
        files.forEach { id -> key(id) { FileCardView(id, core) } }
    }
}

@Composable
private fun FileCardView(id: FileId, core: ClientCore) {
    var loads by remember { mutableIntStateOf(0) }
    val card by produceState<FileCard>(FileCard.Loading, id, loads) { value = core.fileCard(id) }
    when (val current = card) {
        FileCard.Loading -> CircularProgressIndicator(Modifier.size(WontonTheme.size.icon))
        is FileCard.Failed -> Row(
            Modifier.clickable { loads++ },
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            val danger = WontonTheme.colors.danger.toColor()
            Icon(
                WontonIcon.Warning,
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconSmall),
                tint = danger,
            )
            Text(current.message, style = WontonTheme.typography.supporting, color = danger)
        }
        is FileCard.Ready -> ReadyCard(current.info, core)
    }
}

/** 按核心给的 `FileCardActions` 画按钮。 */
@Composable
private fun ReadyCard(info: FileInfo, core: ClientCore) {
    val files = LocalFiles.current
    val context = LocalContext.current
    val actions = remember(info, files.transfers) { core.fileCardActions(info) }
    var playing by remember { mutableStateOf(false) }
    // 播放状态和 Dialog 都在这一行里：行被列表回收（转屏、新消息把它挤出屏幕），播放器就关了，要从头看。放着的时候固定住这一行。
    val pinnable = LocalPinnableContainer.current
    DisposableEffect(playing, pinnable) {
        val pin = if (playing) pinnable?.pin() else null
        onDispose { pin?.release() }
    }
    // 点了还没下载的文件：下完直接打开，不用再点一次。
    var opensWhenDone by remember { mutableStateOf(false) }
    val perform = { action: FileCardAction ->
        when (action) {
            FileCardAction.Play -> playing = true
            FileCardAction.Download -> {
                opensWhenDone = true
                files.onAction(Action.DownloadFile(info.id))
            }
            is FileCardAction.Open -> openWithSystem(context, action.path, info.mimeType)?.let(files.onReject) ?: Unit
            FileCardAction.Wait -> Unit
        }
    }
    val fetch = actions.openElsewhere ?: actions.tap
    LaunchedEffect(fetch) {
        if (opensWhenDone && fetch !is FileCardAction.Wait) {
            opensWhenDone = false
            if (fetch is FileCardAction.Open) perform(fetch)
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Row(
            Modifier.clickable { perform(actions.tap) },
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FileIcon(info, actions.tap == FileCardAction.Play, core)
            Column(Modifier.widthIn(max = 200.dp), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(info.name, style = WontonTheme.typography.supporting, maxLines = 2)
                info.size?.let { SmallText(sizeText(context, it)) }
                sourceText(info.source)?.let { SmallText(it) }
                DownloadProgress(actions.download)
            }
        }
        actions.openElsewhere?.let { elsewhere ->
            Text(
                stringResource(R.string.file_open_elsewhere),
                Modifier.clickable(enabled = elsewhere != FileCardAction.Wait, role = Role.Button) {
                    perform(elsewhere)
                },
                style = WontonTheme.typography.supporting.copy(textDecoration = TextDecoration.Underline),
            )
        }
    }
    if (playing) {
        FilePlayer(
            info,
            core,
            onClose = { playing = false },
            onOpenElsewhere = {
                playing = false
                perform(fetch)
            },
        )
    }
}

@Composable
private fun FileIcon(info: FileInfo, playable: Boolean, core: ClientCore) {
    val thumbnail by produceState<ImageBitmap?>(null, info.id, info.hasThumbnail) {
        if (info.hasThumbnail) value = loadThumbnail(core, info.id)
    }
    val shape = Modifier.size(WontonTheme.size.fileThumbnail).clip(RoundedCornerShape(WontonTheme.radii.control))
    val loaded = thumbnail
    if (loaded != null) {
        Image(loaded, contentDescription = null, shape, contentScale = ContentScale.Crop)
    } else {
        Box(shape, contentAlignment = Alignment.Center) {
            Icon(
                if (playable) WontonIcon.Play else WontonIcon.File,
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.icon),
            )
        }
    }
}

/** 缩略图只是装饰：取不到时画图标，卡片的名字和大小照常显示。 */
private suspend fun loadThumbnail(core: ClientCore, id: FileId): ImageBitmap? =
    try {
        val bytes = core.fileThumbnail(id)
        withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: CoreException) {
        null
    }

@Composable
private fun DownloadProgress(download: Transfer?) {
    val small = WontonTheme.typography.caption
    val danger = WontonTheme.colors.danger.toColor()
    when (val state = download?.state) {
        TransferState.Running -> TransferBar(download)
        TransferState.Paused -> {
            val because = download.pausedBecause
            if (because != null) {
                Text(stringResource(R.string.file_paused, because), style = small, color = danger)
            } else {
                SmallText(stringResource(R.string.file_paused_waiting))
            }
        }
        is TransferState.Failed -> Text(state.message, style = small, color = danger)
        TransferState.Done, is TransferState.LinkNotSent, null -> Unit
    }
}

@Composable
private fun SmallText(text: String) {
    Text(text, Modifier.alpha(WontonTheme.ratio.secondaryContent), style = WontonTheme.typography.caption)
}

/** 还不知道总大小时不确定。 */
@Composable
fun TransferBar(transfer: Transfer) {
    val total = transfer.totalBytes
    if (total != null && total > 0u) {
        LinearProgressIndicator(
            progress = { transfer.transferredBytes.toFloat() / total.toFloat() },
            Modifier.fillMaxWidth(),
        )
    } else {
        LinearProgressIndicator(Modifier.fillMaxWidth())
    }
}

@Composable
private fun sourceText(source: FileSource): String? =
    when (source) {
        FileSource.Device -> null
        is FileSource.Bot -> stringResource(R.string.file_source_bot, source.name, source.plugin)
        FileSource.Unknown -> stringResource(R.string.file_source_unknown)
    }

/** minSdk 29 起按 SI（1000）进位，和 iOS 的 `ByteCountFormatter(.file)` 一致。 */
fun sizeText(context: Context, bytes: ULong): String = Formatter.formatShortFileSize(context, bytes.toLong())
