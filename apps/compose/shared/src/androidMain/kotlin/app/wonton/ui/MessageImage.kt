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
import app.wonton.protocol.SendState
import app.wonton.ui.design.WontonTheme
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun MessageImage(
    image: ImageContent,
    core: ClientCore,
    sendState: SendState,
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
        ImageViewer(image, core) { showingViewer = false }
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
            state.value = try {
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
        }
    }
