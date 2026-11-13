package app.wonton.ui

import android.content.Context
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.AttachmentKind
import app.wonton.protocol.BotView
import app.wonton.protocol.CoreException
import app.wonton.protocol.PendingShare
import app.wonton.protocol.RoomRow
import app.wonton.protocol.ShareInDestinations
import app.wonton.protocol.ShareInImage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.Connection
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WontonIcon
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * 系统分享进来的分享页：核心给条目和去向，主人选聊天点发送。发出去后打开这个聊天看进度；关掉等于不发了。
 * 没配对时点“配对”先收起，配好以后去向变了再出来。
 */
@Composable
internal fun ShareInHost(
    core: ClientCore,
    intake: ShareIntake,
    bots: List<BotView>,
    plugins: PluginsState,
    onOpenRoom: (String) -> Unit,
) {
    val view by remember(core) { core.shareInUpdates() }.collectAsState(remember(core) { core.shareIn() })
    intake.problem?.let { ProblemDialog(it) { intake.problem = null } }
    val share = view.pending
    if (share == null) {
        if (intake.copying) ShareLoadingSheet()
        return
    }
    var pairingLater by remember(share.id) { mutableStateOf(false) }
    if (pairingLater && share.destinations is ShareInDestinations.NotPaired) return
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var sending by remember(share.id) { mutableStateOf(false) }
    var problem by remember(share.id) { mutableStateOf<Problem?>(null) }
    problem?.let { ProblemDialog(it) { problem = null } }
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    ShareSheet(
        share,
        avatar = { row -> RoomAvatar(row, bots.directBot(row), plugins) },
        sending = sending,
        onPair = { pairingLater = true },
        // 正在发时关掉面板不算“不发了”：东西已经交给附件区，发完照样打开聊天。
        onDismiss = { if (!sending) rejectionOf { core.discardShare(null) }?.let { problem = it } },
        onRemove = { id -> rejectionOf { core.discardShare(id) }?.let { problem = it } },
        onSend = { roomId, body ->
            sending = true
            scope.launch {
                val outcome = runCatching {
                    val images = preparePhotos(context, share)
                    withContext(Dispatchers.IO) { core.sendShare(roomId, body, images) }
                }
                sending = false
                outcome.fold(
                    onSuccess = { onOpenRoom(roomId) },
                    onFailure = { failure ->
                        problem = (failure as? CoreException)?.problem
                            ?: Problem(photoUnreadable, photoTryAnother, failure.message)
                    },
                )
            }
        },
    )
}

/** 核心拒了就给出问题，做成了为空。 */
internal fun rejectionOf(call: () -> Unit): Problem? =
    try {
        call()
        null
    } catch (rejection: CoreException) {
        rejection.problem
    }

/** 照片和从相册选的一样压缩、去掉 EXIF 再交给核心（ADR-0005）；读不了的整次发送失败，不悄悄少发一张。 */
private suspend fun preparePhotos(context: Context, share: PendingShare): List<ShareInImage> =
    share.items.filter { it.kind is AttachmentKind.Image }.map { item ->
        val upload = PhotoUpload.make(context, Uri.fromFile(File(item.path)), item.mimeType)
            .getOrElse { throw IOException(it.message, it) }
        ShareInImage(item.id, upload)
    }

@Composable
private fun ShareLoadingSheet() {
    WontonSheet(onDismiss = {}) {
        Row(
            Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
            horizontalArrangement = Arrangement.Center,
        ) {
            CircularProgressIndicator()
        }
    }
}

@Composable
private fun ShareSheet(
    share: PendingShare,
    avatar: @Composable (RoomRow) -> Unit,
    sending: Boolean,
    onPair: () -> Unit,
    onDismiss: () -> Unit,
    onRemove: (String) -> Unit,
    onSend: (String, String) -> Unit,
) {
    WontonSheet(onDismiss) {
        ShareSheetContent(share, avatar, sending, onPair, onRemove, onSend)
    }
}

/** 分享页面板里的内容；预览直接画它（底部面板是弹窗，截图测试画不出来）。 */
@Composable
internal fun ShareSheetContent(
    share: PendingShare,
    avatar: @Composable (RoomRow) -> Unit,
    sending: Boolean,
    onPair: () -> Unit,
    onRemove: (String) -> Unit,
    onSend: (String, String) -> Unit,
) {
    var text by remember(share.id) { mutableStateOf(share.text) }
    var picked by remember(share.id) { mutableStateOf<String?>(null) }
    var choosing by remember(share.id) { mutableStateOf(false) }
    val ready = share.destinations as? ShareInDestinations.Ready
    val rooms = ready?.rooms.orEmpty()
    val chosen = rooms.firstOrNull { it.roomId == (picked ?: ready?.suggestedRoom) }
    val spacing = WontonTheme.spacing
    Column(
        Modifier.padding(horizontal = spacing.pageInset).padding(bottom = spacing.s16),
        verticalArrangement = Arrangement.spacedBy(spacing.s12),
    ) {
        ShareDestination(share.destinations, chosen, avatar, onChoose = { choosing = !choosing }, onPair)
        if (choosing && rooms.isNotEmpty()) {
            ShareRoomChoices(rooms, avatar) { room ->
                picked = room.roomId
                choosing = false
            }
        }
        if (share.items.isNotEmpty()) ShareItemStrip(share.items, onRemove)
        if (share.dropped > 0u) {
            val dropped = stringResource(R.string.share_in_dropped, share.dropped.toInt())
            WontonNotice(WontonIcon.Info, dropped, isError = false)
        }
        WontonTextField(
            text,
            { text = it },
            Modifier.fillMaxWidth(),
            placeholder = stringResource(R.string.composer_message),
            maxLines = MAX_LINES,
        )
        WontonButton(
            stringResource(R.string.composer_send),
            onClick = { chosen?.let { onSend(it.roomId, text) } },
            modifier = Modifier.fillMaxWidth(),
            enabled = chosen != null && !sending && (text.isNotBlank() || share.items.isNotEmpty()),
        )
    }
}

@Composable
private fun ShareDestination(
    destinations: ShareInDestinations,
    chosen: RoomRow?,
    avatar: @Composable (RoomRow) -> Unit,
    onChoose: () -> Unit,
    onPair: () -> Unit,
) {
    when (destinations) {
        ShareInDestinations.NotPaired -> WontonNotice(
            WontonIcon.Connection, stringResource(R.string.share_in_not_paired), isError = false,
        ) {
            WontonButton(stringResource(R.string.share_in_pair), onPair, kind = WontonButtonKind.SECONDARY)
        }
        ShareInDestinations.Loading -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
            CircularProgressIndicator()
        }
        is ShareInDestinations.Unavailable -> ProblemNotice(
            Problem(titleRes = R.string.share_in_chat_unavailable, detail = destinations.message),
        )
        is ShareInDestinations.Ready -> if (destinations.rooms.isEmpty()) {
            WontonNotice(WontonIcon.Chat, stringResource(R.string.share_in_no_chats), isError = false)
        } else {
            ShareRoomRow(chosen, avatar, onChoose)
        }
    }
}

@Composable
private fun ShareRoomChoices(rooms: List<RoomRow>, avatar: @Composable (RoomRow) -> Unit, onPick: (RoomRow) -> Unit) {
    LazyColumn(Modifier.heightIn(max = CHOICES_HEIGHT)) {
        itemsIndexed(rooms, key = { _, row -> row.roomId }) { index, row ->
            if (index > 0) WontonSeparator()
            ShareRoomRow(row, avatar) { onPick(row) }
        }
    }
}

private const val MAX_LINES = 4
private val CHOICES_HEIGHT = 280.dp
