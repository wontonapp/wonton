package app.wonton.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AttachmentFailure
import app.wonton.protocol.AttachmentKind
import app.wonton.protocol.AttachmentState
import app.wonton.protocol.ComposerAttachment
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.File
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Pause
import app.wonton.ui.icons.WarningCircleFill
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 输入框上方的附件区（ADR-0094）：一排小格，进来就开始传。每格右上角能拿掉，传不上去的点开重试。 */
@Composable
internal fun ComposerTray(
    attachments: List<ComposerAttachment>,
    roomId: String,
    core: ClientCore,
    onAction: (Action) -> Unit,
) {
    if (attachments.isEmpty()) return
    LazyRow(
        Modifier.fillMaxWidth().testTag("composerTray"),
        contentPadding = PaddingValues(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        items(attachments, key = { it.id }) { attachment ->
            AttachmentTile(
                attachment, roomId, core, onAction,
                onRemove = {
                    onAction(
                        Action.Timeline(
                            TimelineAction.RemoveAttachment(TimelineTarget(roomId, null), attachment.id),
                        ),
                    )
                },
            )
        }
    }
}

/** 一个附件：照片画预览图，文件画图标和名字，上面盖着上传的进展。附件区和待发消息共用；[onRemove] 为空时不画拿掉的按钮。 */
@Composable
internal fun AttachmentTile(
    attachment: ComposerAttachment,
    roomId: String,
    core: ClientCore,
    onAction: (Action) -> Unit,
    onRemove: (() -> Unit)? = null,
) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    var showingFailure by remember { mutableStateOf(false) }
    Box(
        Modifier.size(WontonTheme.size.previewIcon)
            .clip(shape)
            .background(colors.surfaceSubtle.toColor())
            .semantics { contentDescription = attachment.name },
        contentAlignment = Alignment.Center,
    ) {
        when (attachment.kind) {
            is AttachmentKind.Image -> AttachmentPreview(attachment, roomId, core)
            AttachmentKind.File -> FileFace(attachment.name)
        }
        AttachmentProgress(attachment.state, onFailedClick = { showingFailure = true })
        onRemove?.let { remove ->
            val label = stringResource(R.string.composer_attachment_remove, attachment.name)
            Box(
                Modifier.align(Alignment.TopEnd)
                    .clickable(role = Role.Button, onClick = remove)
                    .semantics { contentDescription = label }
                    .padding(WontonTheme.spacing.s4)
                    .background(Color.Black.copy(alpha = WontonTheme.ratio.scrim), CircleShape)
                    .padding(WontonTheme.spacing.s4),
            ) {
                Icon(
                    WontonIcon.Close,
                    // 装饰：外层的可点区域已经带了说明。
                    contentDescription = null,
                    Modifier.size(WontonTheme.size.iconTiny),
                    tint = Color.White,
                )
            }
        }
    }
    val state = attachment.state
    if (showingFailure && state is AttachmentState.Failed) {
        AttachmentFailureSheet(
            state,
            onRetry = {
                onAction(
                    Action.Timeline(TimelineAction.RetryAttachment(TimelineTarget(roomId, null), attachment.id)),
                )
            },
            onRemove = onRemove,
            onDismiss = { showingFailure = false },
        )
    }
}

@Composable
private fun AttachmentPreview(attachment: ComposerAttachment, roomId: String, core: ClientCore) {
    val preview by produceState<ImageBitmap?>(null, attachment.id) {
        value = withContext(Dispatchers.IO) {
            // 取不到预览就画图片图标：上传的进展和失败照样画在上面。
            runCatching {
                val bytes = core.attachmentThumbnail(TimelineTarget(roomId, null), attachment.id)
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
            }.getOrNull()
        }
    }
    val bitmap = preview
    if (bitmap != null) {
        Image(bitmap, contentDescription = null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
    } else {
        Icon(
            WontonIcon.Image,
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconSmall),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun FileFace(name: String) {
    val tint = WontonTheme.colors.textSecondary.toColor()
    Column(
        Modifier.padding(WontonTheme.spacing.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Icon(WontonIcon.File, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
        Text(
            name,
            style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
            color = tint,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 传的整个过程都画着：有比例画进度环，不知道总量时转圈；断了画暂停，失败画警告，点开说明和重试。 */
@Composable
private fun AttachmentProgress(state: AttachmentState, onFailedClick: () -> Unit) {
    val scrim = Modifier.fillMaxSize().background(Color.Black.copy(alpha = WontonTheme.ratio.scrim))
    when (state) {
        is AttachmentState.Uploading -> Box(scrim, contentAlignment = Alignment.Center) {
            val progress = state.progress
            val ring = Modifier.size(WontonTheme.size.iconSmall + WontonTheme.spacing.s8)
            if (progress != null) {
                CircularProgressIndicator(
                    progress = { progress.coerceIn(0f, 1f) },
                    modifier = ring,
                    color = Color.White,
                    trackColor = Color.White.copy(alpha = WontonTheme.ratio.scrim),
                )
            } else {
                CircularProgressIndicator(ring, color = Color.White)
            }
        }
        AttachmentState.Paused -> {
            val paused = stringResource(R.string.composer_attachment_paused)
            Box(scrim.semantics { stateDescription = paused }, contentAlignment = Alignment.Center) {
                Icon(
                    WontonIcon.Pause,
                    contentDescription = null,
                    Modifier.size(WontonTheme.size.iconSmall),
                    tint = Color.White,
                )
            }
        }
        AttachmentState.Ready -> Unit
        is AttachmentState.Failed -> {
            val failed = stringResource(R.string.composer_attachment_failed)
            Box(
                scrim.clickable(role = Role.Button, onClick = onFailedClick).semantics { contentDescription = failed },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    WontonIcon.WarningCircleFill,
                    contentDescription = null,
                    Modifier.size(WontonTheme.size.icon),
                    tint = WontonTheme.colors.danger.toColor(),
                )
            }
        }
    }
}

/** 附件传不上去：一句人话说原因，详情收起，配重试和拿掉。 */
@Composable
private fun AttachmentFailureSheet(
    state: AttachmentState.Failed,
    onRetry: () -> Unit,
    onRemove: (() -> Unit)?,
    onDismiss: () -> Unit,
) {
    WontonSheet(onDismiss) { close ->
        Column(
            Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Text(
                stringResource(state.reason.text),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            ProblemDetail(state.message)
            Row(
                Modifier.padding(bottom = WontonTheme.spacing.s8),
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                onRemove?.let { remove ->
                    WontonButton(
                        stringResource(R.string.common_delete),
                        { close(remove) },
                        Modifier.weight(1f),
                        WontonButtonKind.SECONDARY,
                    )
                }
                WontonButton(
                    stringResource(R.string.composer_attachment_retry),
                    { close(onRetry) },
                    Modifier.weight(1f),
                )
            }
        }
    }
}

/** 类别由核心给，说法在壳里。 */
private val AttachmentFailure.text: Int
    get() = when (this) {
        AttachmentFailure.TOO_LARGE -> R.string.composer_attachment_too_large
        AttachmentFailure.SOURCE_UNAVAILABLE -> R.string.composer_attachment_source_unavailable
        AttachmentFailure.STORAGE_FULL -> R.string.composer_attachment_storage_full
        AttachmentFailure.REJECTED -> R.string.composer_attachment_rejected
        AttachmentFailure.UPSTREAM -> R.string.composer_attachment_upstream
        AttachmentFailure.LOCAL_STORAGE -> R.string.composer_attachment_local_storage
        AttachmentFailure.OTHER -> R.string.composer_attachment_failed
    }
