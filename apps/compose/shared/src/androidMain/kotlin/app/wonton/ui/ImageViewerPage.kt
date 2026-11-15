package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.client.ClientCore
import app.wonton.protocol.ImageContent
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 一页：这一张的原图，下载中转圈、取不到给说明。手势状态和重试都是这一页自己的。 */
@Composable
internal fun ViewerPage(
    image: ImageContent,
    page: Int,
    transform: ViewerTransform,
    loaded: MutableState<Result<ImageBitmap>?>,
    formatUnsupported: String,
    photoLabel: String,
    core: ClientCore,
    onDismiss: () -> Unit,
) {
    var attempt by remember(page) { mutableIntStateOf(0) }
    LaunchedEffect(image.media, attempt) {
        loaded.value = null
        loaded.value = decodeImage(core, image.media, formatUnsupported)
    }
    DisposableEffect(image.media) {
        onDispose { loaded.value = null }
    }
    val result = loaded.value
    when {
        result == null -> LoadingOriginal()
        // 原图：文件名当读屏名字，和 iOS 的查看器一致；没有文件名时用“图片”。
        result.isSuccess -> OriginalImage(
            result.getOrThrow(),
            transform,
            onDismiss,
            image.filename.ifEmpty { photoLabel },
        )
        else -> OriginalFailed(result.exceptionOrNull()) { attempt++ }
    }
}

/** 第几张：只有数字的轻量位置，跟顶栏一样高。 */
@Composable
internal fun PositionPill(current: Int, total: Int, onBlack: Boolean, modifier: Modifier = Modifier) {
    val description = stringResource(R.string.message_image_position, current, total)
    Box(modifier, contentAlignment = Alignment.Center) {
        Text(
            "$current/$total",
            Modifier.semantics { contentDescription = description },
            style = WontonTheme.typography.body,
            color = if (onBlack) Color.White else WontonTheme.colors.textPrimary.toColor(),
        )
    }
}
