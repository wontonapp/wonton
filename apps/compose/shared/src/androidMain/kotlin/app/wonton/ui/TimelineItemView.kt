package app.wonton.ui

import android.graphics.ImageDecoder
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.MessageContent
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import java.io.IOException
import java.nio.ByteBuffer
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** `showsSender`：私聊里对方是谁看标题就知道，不在每条消息上重复名字。`onRetry`、`onCancel` 处理发送失败的这一条。 */
@Composable
fun TimelineItemView(
    item: TimelineItem,
    core: ClientCore,
    showsSender: Boolean,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
) {
    when (val content = item.content) {
        is TimelineContent.Message -> MessageBubble(content.message, core, showsSender, onRetry, onCancel)
        is TimelineContent.Notice -> CenteredNote(content.text)
        is TimelineContent.DateDivider ->
            CenteredNote(DateFormat.getDateInstance(DateFormat.FULL).format(Date(content.timestampMs.toLong())))
        TimelineContent.ReadMarker -> Row(verticalAlignment = Alignment.CenterVertically) {
            HorizontalDivider(Modifier.weight(1f))
            Text(
                "以下是新消息",
                Modifier.padding(horizontal = 8.dp),
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.labelSmall,
            )
            HorizontalDivider(Modifier.weight(1f))
        }
        TimelineContent.TimelineStart -> CenteredNote("以上是聊天的开头")
    }
}

@Composable
private fun CenteredNote(text: String) {
    Text(
        text,
        Modifier.fillMaxWidth(),
        textAlign = TextAlign.Center,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun MessageBubble(
    message: TimelineMessage,
    core: ClientCore,
    showsSender: Boolean,
    onRetry: () -> Unit,
    onCancel: () -> Unit,
) {
    val alignment = if (message.isOwn) Alignment.End else Alignment.Start
    Column(Modifier.fillMaxWidth(), horizontalAlignment = alignment) {
        if (showsSender && !message.isOwn) {
            Text(message.senderName, style = MaterialTheme.typography.labelSmall)
        }
        val background = if (message.isOwn) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        }
        Box(
            Modifier.widthIn(max = 280.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(background)
                .padding(10.dp),
        ) {
            MessageBody(message.body, core)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            val small = MaterialTheme.typography.labelSmall
            if (message.edited) Text("已编辑", style = small)
            Text(DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(message.timestampMs.toLong())), style = small)
            when (val state = message.sendState) {
                SendState.Sending -> Text("发送中", style = small)
                is SendState.Failed -> FailedSend(state, small, onRetry, onCancel)
                SendState.Sent -> if (message.isOwn && message.readBy.isNotEmpty()) Text("已读", style = small)
            }
        }
    }
}

/** 点开选重发或删除。 */
@Composable
private fun FailedSend(state: SendState.Failed, style: TextStyle, onRetry: () -> Unit, onCancel: () -> Unit) {
    var choosing by remember { mutableStateOf(false) }
    Box {
        Text(
            if (state.recoverable) "发送失败，会自动重试" else "发送失败：${state.message}",
            Modifier.clickable { choosing = true },
            style = style,
            color = MaterialTheme.colorScheme.error,
        )
        DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
            DropdownMenuItem(
                text = { Text("重发") },
                onClick = {
                    choosing = false
                    onRetry()
                },
            )
            DropdownMenuItem(
                text = { Text("删除", color = MaterialTheme.colorScheme.error) },
                onClick = {
                    choosing = false
                    onCancel()
                },
            )
        }
    }
}

@Composable
private fun MessageBody(body: MessageContent, core: ClientCore) {
    when (body) {
        is MessageContent.Text -> SelectionContainer { Text(body.body) }
        is MessageContent.Image -> MessageImage(body.image, core)
        MessageContent.Undecryptable -> Text("无法解密：本设备还没有这条消息的密钥", fontStyle = FontStyle.Italic)
        MessageContent.Redacted -> Text("消息已撤回", fontStyle = FontStyle.Italic)
        is MessageContent.Unsupported -> Text("暂不支持显示的消息（${body.kind}）", fontStyle = FontStyle.Italic)
    }
}

@Composable
private fun MessageImage(image: ImageContent, core: ClientCore) {
    val shown = image.thumbnail ?: image.media
    val preview by loadedImage(core, shown)
    var showingOriginal by remember { mutableStateOf(false) }
    Box(
        Modifier.widthIn(max = 240.dp)
            .aspectRatio(image.ratio())
            .clip(RoundedCornerShape(8.dp))
            .clickable { showingOriginal = true },
        contentAlignment = Alignment.Center,
    ) {
        LoadedImage(preview, ContentScale.Crop)
    }
    if (showingOriginal) {
        Dialog(
            onDismissRequest = { showingOriginal = false },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            val original by loadedImage(core, image.media)
            Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
                LoadedImage(original, ContentScale.Fit)
                TextButton(onClick = { showingOriginal = false }, Modifier.align(Alignment.TopEnd).padding(8.dp)) {
                    Text("关闭", color = Color.White)
                }
            }
        }
    }
}

private fun ImageContent.ratio(): Float {
    val width = width?.toFloat() ?: 0f
    val height = height?.toFloat() ?: 0f
    return if (width > 0f && height > 0f) width / height else 1f
}

@Composable
private fun LoadedImage(state: Result<ImageBitmap>?, scale: ContentScale) {
    when {
        state == null -> CircularProgressIndicator()
        state.isSuccess ->
            Image(state.getOrThrow(), contentDescription = null, Modifier.fillMaxSize(), contentScale = scale)
        else -> Text(state.exceptionOrNull()?.message ?: "取不到图片", Modifier.padding(8.dp))
    }
}

/** `ImageDecoder` 解码时按 EXIF 方向转正（`BitmapFactory` 不会），别的客户端发来、只靠 EXIF 标方向的照片也是正的。 */
@Composable
private fun loadedImage(core: ClientCore, media: MediaHandle) =
    remember(media) { mutableStateOf<Result<ImageBitmap>?>(null) }.also { state ->
        LaunchedEffect(media) {
            state.value = try {
                val bytes = core.loadMedia(media)
                withContext(Dispatchers.Default) {
                    val source = ImageDecoder.createSource(ByteBuffer.wrap(bytes))
                    Result.success(ImageDecoder.decodeBitmap(source).asImageBitmap())
                }
            } catch (failure: CoreException) {
                Result.failure(IllegalStateException(failure.toString(), failure))
            } catch (failure: IOException) {
                Result.failure(IllegalStateException("图片格式不支持", failure))
            }
        }
    }
