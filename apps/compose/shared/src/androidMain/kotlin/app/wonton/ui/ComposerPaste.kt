package app.wonton.ui

import android.content.ContentResolver
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.consume
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ImageUpload
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 预览位图的最长边：比大多数屏幕高，又不至于为大图解码整块原始像素。 */
private const val PREVIEW_MAX_PIXELS = 1280f

private val PREVIEW_MAX_HEIGHT = 280.dp

/**
 * 输入框收到的内容（长按粘贴、Ctrl+V、键盘的图片插入、拖放）。带正文的条目是链接或富文本，照常粘成文字；
 * 只带 URI 的条目按类型交给 [onImage] 和 [onFile]。一次剪贴板里有多张图时只取第一张：预览一次只放得下一张，
 * 余下的条目不消费，也不悄悄丢。
 */
@OptIn(ExperimentalFoundationApi::class)
internal fun pasteContentListener(
    resolver: ContentResolver,
    onImage: (Uri) -> Unit,
    onFile: (Uri) -> Unit,
): ReceiveContentListener = ReceiveContentListener { transferable ->
    val clip = transferable.clipEntry?.clipData ?: return@ReceiveContentListener transferable
    var handled = false
    transferable.consume { item ->
        if (handled) return@consume false
        val uri = item.uri ?: return@consume false
        if (item.text != null) return@consume false
        handled = true
        // 有的来源（如键盘插入）URI 上查不到类型，剪贴板描述里带；描述只有通配符时当文件。
        val mime = runCatching { resolver.getType(uri) }.getOrNull()
            ?: clip.description.filterMimeTypes("image/*").firstOrNull()
        if (mime?.startsWith("image/") == true) onImage(uri) else onFile(uri)
        true
    }
}

/** 粘贴的图片先在这里过目：确认才发送，取消就丢掉。转码没好之前显示转圈，读不了就报错关掉。 */
@Composable
internal fun PastePreviewSheet(
    uri: Uri,
    onSend: (ImageUpload) -> Unit,
    onDismiss: () -> Unit,
    onReject: (Problem) -> Unit,
) {
    val context = LocalContext.current
    // 字符串资源在协程里取不到，报错前先取好。
    val photoUnreadable = stringResource(R.string.photo_unreadable)
    val photoTryAnother = stringResource(R.string.photo_try_another)
    var preview by remember(uri) { mutableStateOf<Result<PastePreview>?>(null) }
    LaunchedEffect(uri) {
        preview = withContext(Dispatchers.Default) {
            runCatching {
                val upload = PhotoUpload.make(context, uri).getOrThrow()
                PastePreview(upload, previewBitmap(upload))
            }
        }
    }
    LaunchedEffect(preview) {
        preview?.exceptionOrNull()?.let { failure ->
            onReject(Problem(photoUnreadable, photoTryAnother, failure.message))
            onDismiss()
        }
    }
    WontonSheet(onDismiss = onDismiss) { close ->
        val ready = preview?.getOrNull()
        if (ready == null) {
            Box(
                Modifier.fillMaxWidth().heightIn(min = 160.dp),
                contentAlignment = Alignment.Center,
            ) { CircularProgressIndicator() }
        } else {
            Column(
                Modifier.padding(horizontal = WontonTheme.spacing.pageInset)
                    .verticalScroll(rememberScrollState()),
            ) {
                ready.bitmap?.let { bitmap ->
                    Image(
                        bitmap,
                        null,
                        Modifier.fillMaxWidth()
                            .heightIn(max = PREVIEW_MAX_HEIGHT)
                            .clip(RoundedCornerShape(WontonTheme.radii.card)),
                        contentScale = ContentScale.Fit,
                    )
                }
                Row(Modifier.padding(vertical = WontonTheme.spacing.s8)) {
                    WontonButton(
                        stringResource(R.string.common_cancel),
                        { close {} },
                        Modifier.weight(1f),
                        WontonButtonKind.SECONDARY,
                    )
                    WontonButton(
                        stringResource(R.string.composer_send),
                        { close { onSend(ready.upload) } },
                        Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

/** 过目的内容和显示用的位图一起留着，确认发送时才把 [upload] 交给核心。 */
private class PastePreview(val upload: ImageUpload, val bitmap: ImageBitmap?)

/** `ImageDecoder` 按 EXIF 方向转正，缩到预览的尺寸；解不出来时预览没有图，发送不受影响。 */
private fun previewBitmap(upload: ImageUpload): ImageBitmap? = runCatching {
    val bytes = upload.thumbnail?.data ?: upload.data
    ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
        // 要在 CPU 上读像素，不能用硬件位图。
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        val scale = min(1f, PREVIEW_MAX_PIXELS / max(info.size.width, info.size.height))
        decoder.setTargetSize(
            (info.size.width * scale).toInt().coerceAtLeast(1),
            (info.size.height * scale).toInt().coerceAtLeast(1),
        )
    }.asImageBitmap()
}.getOrNull()
