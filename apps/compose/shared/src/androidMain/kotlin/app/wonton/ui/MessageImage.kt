package app.wonton.ui

import android.graphics.ImageDecoder
import androidx.compose.foundation.Image
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.LocalPinnableContainer
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.MessageContent
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.ui.design.WontonTheme
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * 消息里的图片。长按给消息菜单；[groupImages] 是这条消息所在一组（ADR-0094）的图片、[groupIndex] 是这张在组里的
 * 位置，查看器拿它们左右翻页；不在一组里（长按菜单的预览拿不到整条时间线）就只看这一张。
 */
@Composable
fun MessageImage(
    image: ImageContent,
    core: ClientCore,
    sendState: SendState,
    groupImages: List<ImageContent> = emptyList(),
    groupIndex: Int = 0,
    onLongPress: (() -> Unit)? = null,
) {
    val shown = image.thumbnail ?: image.media
    val preview by loadedImage(core, shown)
    var showingViewer by remember { mutableStateOf(false) }
    // 查看器开着时钉住这一行：行被列表回收，查看器会跟着一起被关掉。
    val pinnable = LocalPinnableContainer.current
    DisposableEffect(showingViewer, pinnable) {
        val pin = if (showingViewer) pinnable?.pin() else null
        onDispose { pin?.release() }
    }
    Box(
        Modifier.widthIn(max = WontonTheme.size.messageImage)
            .aspectRatio(image.ratio())
            .clip(RoundedCornerShape(WontonTheme.radii.control))
            // 点开看图；长按给消息菜单（图片盖满气泡，菜单的手势要从这里来）。
            .combinedClickable(onClick = { showingViewer = true }, onLongClick = { onLongPress?.invoke() }),
        contentAlignment = Alignment.Center,
    ) {
        LoadedImage(preview, ContentScale.Crop, stringResource(R.string.message_quote_image))
        if (sendState is SendState.Sending) {
            val bar = Modifier.align(Alignment.BottomStart).fillMaxWidth()
            val progress = sendState.progress
            if (progress == null) LinearProgressIndicator(bar)
            else LinearProgressIndicator({ progress }, bar)
        }
    }
    if (showingViewer) {
        // 不在一组里时这组就是它自己：查看器按一张显示。
        val images = groupImages.ifEmpty { listOf(image) }
        ImageViewer(images, groupIndex.coerceIn(images.indices), core) { showingViewer = false }
    }
}

private fun ImageContent.ratio(): Float {
    val width = width?.toFloat() ?: 0f
    val height = height?.toFloat() ?: 0f
    return if (width > 0f && height > 0f) width / height else 1f
}

@Composable
private fun LoadedImage(state: Result<ImageBitmap>?, scale: ContentScale, label: String) {
    val unavailable = stringResource(R.string.message_image_unavailable)
    when {
        state == null -> CircularProgressIndicator()
        // 整张图就是消息内容，点开还有查看器，读屏要有名字。
        state.isSuccess ->
            Image(state.getOrThrow(), contentDescription = label, Modifier.fillMaxSize(), contentScale = scale)
        else -> Text(
            (state.exceptionOrNull() as? CoreException)?.problem?.titleText
                ?: state.exceptionOrNull()?.message ?: unavailable,
            Modifier.padding(WontonTheme.spacing.s8),
            style = WontonTheme.typography.supporting,
        )
    }
}

/** `ImageDecoder` 解码时按 EXIF 方向转正（`BitmapFactory` 不会），别的客户端发来、只靠 EXIF 标方向的照片也是正的。
 *  [retryKey] 变了就重新取一次：查看器用它做重试。 */
@Composable
internal fun loadedImage(core: ClientCore, media: MediaHandle, retryKey: Int = 0) =
    remember(media) { mutableStateOf<Result<ImageBitmap>?>(null) }.also { state ->
        val formatUnsupported = stringResource(R.string.message_image_format_unsupported)
        LaunchedEffect(media, retryKey) {
            state.value = null
            state.value = decodeImage(core, media, formatUnsupported)
        }
    }

/** 取媒体并按 EXIF 方向解码；本地解不开的（IOException）按“格式不支持”报，核心的错误原样给。 */
internal suspend fun decodeImage(
    core: ClientCore,
    media: MediaHandle,
    formatUnsupported: String,
): Result<ImageBitmap> = try {
    val bytes = core.loadMedia(media)
    withContext(Dispatchers.Default) {
        val source = ImageDecoder.createSource(ByteBuffer.wrap(bytes))
        Result.success(ImageDecoder.decodeBitmap(source).asImageBitmap())
    }
} catch (failure: CoreException) {
    Result.failure(failure)
} catch (failure: IOException) {
    Result.failure(IllegalStateException(formatUnsupported, failure))
}

/**
 * 时间线第 [index] 条消息所在一组（ADR-0094 的分组标记）的图片，按显示顺序（早的在前），和这张在组里的位置；
 * 图片查看器左右翻整组用。不带标记的图片消息一组就是它自己；挨着才算一组：中间隔了别人的消息就断开。
 */
internal fun imageGroup(items: List<TimelineItem>, index: Int): Pair<List<ImageContent>, Int> {
    val message = (items[index].content as? TimelineContent.Message)?.message
        ?: return emptyList<ImageContent>() to 0
    val groupId = message.group?.id
    var start = index
    var end = index
    if (groupId != null) {
        fun inGroup(other: TimelineItem): Boolean {
            val member = (other.content as? TimelineContent.Message)?.message ?: return false
            return member.senderId == message.senderId && member.group?.id == groupId
        }
        while (start > 0 && inGroup(items[start - 1])) start--
        while (end + 1 < items.size && inGroup(items[end + 1])) end++
    }
    // 时间线是新到旧排的：从 end 往回数才是显示顺序（早的在前）。
    val images = ArrayList<ImageContent>()
    var position = 0
    for (i in end downTo start) {
        val image = ((items[i].content as? TimelineContent.Message)?.message?.body as? MessageContent.Image)?.image
            ?: continue
        if (i == index) position = images.size
        images.add(image)
    }
    return images to position
}
