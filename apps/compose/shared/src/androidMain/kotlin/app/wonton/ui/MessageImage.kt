package app.wonton.ui

import android.graphics.ImageDecoder
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.SendState
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun MessageImage(image: ImageContent, core: ClientCore, sendState: SendState) {
    val shown = image.thumbnail ?: image.media
    val preview by loadedImage(core, shown)
    var showingOriginal by remember { mutableStateOf(false) }
    Box(
        Modifier.widthIn(max = WontonTheme.size.messageImage)
            .aspectRatio(image.ratio())
            .clip(RoundedCornerShape(WontonTheme.radii.control))
            .clickable { showingOriginal = true },
        contentAlignment = Alignment.Center,
    ) {
        LoadedImage(preview, ContentScale.Crop)
        if (sendState is SendState.Sending) {
            val bar = Modifier.align(Alignment.BottomStart).fillMaxWidth()
            val progress = sendState.progress
            if (progress == null) LinearProgressIndicator(bar)
            else LinearProgressIndicator({ progress }, bar)
        }
    }
    if (showingOriginal) {
        Dialog(
            onDismissRequest = { showingOriginal = false },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            val original by loadedImage(core, image.media)
            Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
                LoadedImage(original, ContentScale.Fit)
                Box(Modifier.align(Alignment.TopEnd).padding(WontonTheme.spacing.s8)) {
                    WontonIconButton(WontonIcon.Close, "关闭", { showingOriginal = false }, Color.White)
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
        else -> Text(
            state.exceptionOrNull()?.message ?: "取不到图片",
            Modifier.padding(WontonTheme.spacing.s8),
            style = WontonTheme.typography.supporting,
        )
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
                Result.failure(IllegalStateException(failure.problem.title, failure))
            } catch (failure: IOException) {
                Result.failure(IllegalStateException("图片格式不支持", failure))
            }
        }
    }
