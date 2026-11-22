package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.lerp
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.util.lerp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val CLOSE_DRAG_THRESHOLD = 120.dp
private const val HINT_MS = 2_500L

/**
 * 图片的全屏查看器：原图、双指缩放、双击放大/还原、放大后拖动、下滑关闭；顶栏存进相册和分享。
 * `group` 是同一次发送（ADR-0094）里的几张照片：有多张时左右滑、点边区换张，顶栏显示页码；单张或组里认不出就只看这一张。
 * 原图还在下载时转圈，取不到时给出可重试的说明。
 */
@Composable
internal fun ImageViewer(image: ImageContent, group: List<ImageContent>?, core: ClientCore, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        val images = group?.takeIf { candidates -> candidates.any { it.media == image.media } } ?: listOf(image)
        val startIndex = images.indexOfFirst { it.media == image.media }.coerceAtLeast(0)
        ImageViewerContent(images, startIndex, core, onDismiss)
    }
}

@Composable
private fun ImageViewerContent(images: List<ImageContent>, startIndex: Int, core: ClientCore, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val transform = remember(scope, density) {
        ViewerTransform(scope, with(density) { CLOSE_DRAG_THRESHOLD.toPx() })
    }
    // 正在看第几张、哪张图：列表在查看器开着时删掉一张，重建 PageState 也要停在同一张（没了就停原位）。
    var viewing by remember { mutableStateOf<Pair<MediaHandle, Int>?>(null) }
    val pager = rememberPager(images, viewing, startIndex, transform, density, scope)
    val index = pager?.index ?: startIndex
    LaunchedEffect(pager, index, images) { viewing = images.getOrNull(index)?.let { it.media to index } }
    val originals = remember { OriginalCache() }
    val loaded = rememberOriginal(core, images[index].media, originals)
    var saving by remember { mutableStateOf(false) }
    var hint by remember { mutableStateOf(0) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    LaunchedEffect(hint) { if (hint != 0) { delay(HINT_MS); hint = 0 } }
    val current = images[index]

    val save: () -> Unit = {
        if (!saving) {
            saving = true
            scope.launch {
                problem = saveToGallery(context, core, current)
                if (problem == null) hint = R.string.message_image_saved
                saving = false
            }
        }
    }
    val share: () -> Unit = {
        scope.launch { problem = shareWithOthers(context, core, current) }
    }

    val failed = loaded != null && loaded.isFailure
    val pageText = pager?.let { stringResource(R.string.message_image_page, index + 1, it.count) }
    Box(
        Modifier.fillMaxSize()
            .testTag("viewer")
            .onSizeChanged { transform.viewport = it }
            .graphicsLayer {
                // 跟手下拉时整层淡出，松手够远就关。
                alpha = 1f - (transform.dismiss.y / transform.viewport.height.coerceAtLeast(1))
                    .coerceIn(0f, 1f)
            }
            .background(Color.Black),
    ) {
        // 手势铺满整屏（黑边也算），放在页面内容下面：按钮是更深的节点，Main 轮里先拿到事件，
        // 这里看到已消费的事件就退出，不跟按钮抢。
        Box(
            Modifier.fillMaxSize()
                .pointerInput(pager) { viewerGestures(transform, pager, onDismiss) },
        )
        PageLayer(images[index], loaded, transform, pager?.drag ?: 0f) {
            originals.retry(images[index].media)
        }
        pager?.neighborIndex?.let { neighbor ->
            val neighborLoaded = rememberOriginal(core, images[neighbor].media, originals)
            PageLayer(
                images[neighbor],
                neighborLoaded,
                transform = null,
                slideX = pager.drag + pager.side * transform.viewport.width,
            ) { originals.retry(images[neighbor].media) }
        }
        TopBar(
            onClose = onDismiss,
            saving = saving,
            onSave = save,
            onShare = share,
            onBlack = !failed,
            pageText = pageText,
        )
        if (hint != 0) HintPill(stringResource(hint))
    }
    problem?.let { ProblemDialog(it) { problem = null } }
}

/** Android 10+ publishes only the app-created MediaStore entry; no broad storage permission is needed. */
@Suppress("TooGenericExceptionCaught")
private suspend fun saveToGallery(context: Context, core: ClientCore, image: ImageContent): Problem? = try {
    withContext(Dispatchers.IO) {
        val bytes = core.loadMedia(image.media)
        saveImageToGallery(context, bytes, image.filename, image.mimeType)
    }
    null
} catch (failure: CancellationException) {
    throw failure
} catch (failure: Exception) {
    imageOperationProblem(failure, R.string.message_image_save_failed)
}

@Suppress("TooGenericExceptionCaught")
private suspend fun shareWithOthers(context: Context, core: ClientCore, image: ImageContent): Problem? = try {
    val bytes = withContext(Dispatchers.IO) { core.loadMedia(image.media) }
    val (path, mime) = withContext(Dispatchers.IO) {
        shareImageFile(context, bytes, image.filename, image.mimeType)
    }
    shareWithSystem(context, path, mime)
} catch (failure: CancellationException) {
    throw failure
} catch (failure: Exception) {
    imageOperationProblem(failure, R.string.message_image_share_failed)
}


/** 一页：加载中、原图（当前页可缩放）或失败说明。`slideX` 是翻页拖动的整页横向位移，邻页不缩放。 */
@Composable
private fun PageLayer(
    image: ImageContent,
    loaded: Result<ImageBitmap>?,
    transform: ViewerTransform?,
    slideX: Float,
    onRetry: () -> Unit,
) {
    val photo = stringResource(R.string.message_quote_image)
    Box(Modifier.fillMaxSize().graphicsLayer { translationX = slideX }) {
        when {
            loaded == null -> LoadingOriginal()
            loaded.isSuccess -> {
                val label = image.filename.ifEmpty { photo }
                OriginalImage(loaded.getOrThrow(), transform, label)
            }
            else -> OriginalFailed(loaded.exceptionOrNull(), onRetry)
        }
    }
}

@Composable
private fun LoadingOriginal() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(color = Color.White)
    }
}

/** 原图：文件名当读屏名字，和 iOS 的查看器一致；没有文件名时用“图片”。缩放手势在整层上，不在图上。 */
@Composable
private fun OriginalImage(bitmap: ImageBitmap, transform: ViewerTransform?, label: String) {
    Image(
        bitmap,
        contentDescription = label,
        modifier = Modifier
            .fillMaxSize()
            .testTag("viewer-image")
            .graphicsLayer {
                // 邻页没有缩放状态，永远一比一。
                if (transform != null) {
                    scaleX = transform.scale
                    scaleY = transform.scale
                    translationX = transform.offset.x + transform.dismiss.x
                    translationY = transform.offset.y + transform.dismiss.y
                }
            },
        contentScale = ContentScale.Fit,
    )
}

/** 原图取不到：整页说明，可重试；错误的技术细节默认收起。 */
@Composable
private fun OriginalFailed(failure: Throwable?, onRetry: () -> Unit) {
    Surface(Modifier.fillMaxSize(), color = WontonTheme.colors.surface.toColor()) {
        val problem = when (failure) {
            is CoreException -> failure.problem
            else -> Problem(titleRes = R.string.message_image_format_unsupported)
        }
        ProblemView(problem, onRetry)
    }
}

/** 顶栏：左边关闭，中间页码（同组多张时），右边保存和分享，只用图标。 */
@Composable
private fun TopBar(
    onClose: () -> Unit,
    saving: Boolean,
    onSave: () -> Unit,
    onShare: () -> Unit,
    onBlack: Boolean,
    pageText: String?,
) {
    val tint = if (onBlack) Color.White else null
    val textColor = tint ?: WontonTheme.colors.textPrimary.toColor()
    val spinnerColor = textColor
    // 三个孩子叠在一个 Box 里各自对齐：页码真正居中，不随两边按钮组宽度不同而偏。
    Box(
        Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = WontonTheme.spacing.s4),
    ) {
        Box(Modifier.align(Alignment.CenterStart)) {
            WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), onClose, tint)
        }
        if (pageText != null) {
            Text(
                pageText,
                Modifier.align(Alignment.Center).testTag("viewer-page"),
                color = textColor,
                style = WontonTheme.typography.body,
                maxLines = 1,
            )
        }
        Row(
            Modifier.align(Alignment.CenterEnd),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(WontonTheme.size.target), contentAlignment = Alignment.Center) {
                // 保存中按钮不挪走：点空出来的 spinner 会落到手势层，点右三分之一就翻页了。
                WontonIconButton(
                    WontonIcon.Download,
                    stringResource(R.string.message_image_save),
                    onSave,
                    tint,
                )
                if (saving) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.icon),
                        color = spinnerColor,
                        strokeWidth = 2.dp,
                    )
                }
            }
            WontonIconButton(WontonIcon.Share, stringResource(R.string.message_image_share), onShare, tint)
        }
    }
}

/** 轻提示：操作做成或没做成的一句话，几秒后自己消失。 */
@Composable
private fun HintPill(text: String) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
        Surface(
            shape = RoundedCornerShape(WontonTheme.radii.pill),
            color = WontonTheme.colors.surface.toColor(),
            modifier = Modifier.padding(WontonTheme.spacing.s16),
        ) {
            Text(
                text,
                Modifier.padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
        }
    }
}
