package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageContent
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
 * [images] 是一条消息（一组，ADR-0094）的图片、非空，[startIndex] 是打开的那张：左右翻这组，顶栏下显示第几张。
 * 原图还在下载时转圈，取不到时给出可重试的说明。
 */
@Composable
internal fun ImageViewer(images: List<ImageContent>, startIndex: Int, core: ClientCore, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        ImageViewerContent(images, startIndex.coerceIn(images.indices), core, onDismiss)
    }
}

@Composable
private fun ImageViewerContent(images: List<ImageContent>, startIndex: Int, core: ClientCore, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val closeThreshold = with(density) { CLOSE_DRAG_THRESHOLD.toPx() }
    // 每页一份手势状态（缩放、位移、下滑）：这一页翻走了状态还在，翻回来接着用。
    val transforms = remember(images) { List(images.size) { ViewerTransform(scope, closeThreshold) } }
    // 每页的原图加载结果：顶栏按当前页有没有失败换颜色。页面滑出屏幕就放开那张位图，回来再取。
    val loads = remember(images) { List(images.size) { mutableStateOf<Result<ImageBitmap>?>(null) } }
    val formatUnsupported = stringResource(R.string.message_image_format_unsupported)
    val pagerState = rememberPagerState(initialPage = startIndex) { images.size }
    var saving by remember { mutableStateOf(false) }
    var hint by remember { mutableStateOf(0) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    LaunchedEffect(hint) { if (hint != 0) { delay(HINT_MS); hint = 0 } }

    // 存进相册、分享、顶栏颜色都看正在看的那张。
    val current = images[pagerState.currentPage]
    val failed = loads[pagerState.currentPage].value?.isFailure == true
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

    val photo = stringResource(R.string.message_quote_image)
    Box(
        Modifier.fillMaxSize()
            .onSizeChanged { size -> transforms.forEach { it.viewport = size } }
            .graphicsLayer {
                // 跟手下拉时整层淡出，松手够远就关。
                val transform = transforms[pagerState.currentPage]
                alpha = 1f - (transform.dismiss.y / transform.viewport.height.coerceAtLeast(1))
                    .coerceIn(0f, 1f)
            }
            .background(Color.Black),
    ) {
        HorizontalPager(state = pagerState, modifier = Modifier.fillMaxSize()) { page ->
            ViewerPage(images[page], page, transforms[page], loads[page], formatUnsupported, photo, core, onDismiss)
        }
        TopBar(
            onClose = onDismiss,
            saving = saving,
            onSave = save,
            onShare = share,
            onBlack = !failed,
        )
        if (images.size > 1) {
            PositionPill(
                pagerState.currentPage + 1,
                images.size,
                onBlack = !failed,
                Modifier.align(Alignment.TopCenter)
                    .statusBarsPadding()
                    .height(WontonTheme.size.target),
            )
        }
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


@Composable
internal fun LoadingOriginal() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(color = Color.White)
    }
}

/** 原图：文件名当读屏名字，和 iOS 的查看器一致；没有文件名时用“图片”。 */
@Composable
internal fun OriginalImage(bitmap: ImageBitmap, transform: ViewerTransform, onDismiss: () -> Unit, label: String) {
    Image(
        bitmap,
        contentDescription = label,
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(Unit) { viewerGestures(transform, onDismiss) }
            .graphicsLayer {
                scaleX = transform.scale
                scaleY = transform.scale
                translationX = transform.offset.x + transform.dismiss.x
                translationY = transform.offset.y + transform.dismiss.y
            },
        contentScale = ContentScale.Fit,
    )
}

/** 原图取不到：整页说明，可重试；错误的技术细节默认收起。 */
@Composable
internal fun OriginalFailed(failure: Throwable?, onRetry: () -> Unit) {
    Surface(Modifier.fillMaxSize(), color = WontonTheme.colors.surface.toColor()) {
        val problem = when (failure) {
            is CoreException -> failure.problem
            else -> Problem(titleRes = R.string.message_image_format_unsupported)
        }
        ProblemView(problem, onRetry)
    }
}

/** 顶栏：左边关闭，右边保存和分享，只用图标。 */
@Composable
private fun TopBar(
    onClose: () -> Unit,
    saving: Boolean,
    onSave: () -> Unit,
    onShare: () -> Unit,
    onBlack: Boolean,
) {
    val tint = if (onBlack) Color.White else null
    val spinnerColor = tint ?: WontonTheme.colors.textPrimary.toColor()
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), onClose, tint)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(WontonTheme.size.target), contentAlignment = Alignment.Center) {
                if (saving) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.icon),
                        color = spinnerColor,
                        strokeWidth = 2.dp,
                    )
                } else {
                    WontonIconButton(
                        WontonIcon.Download,
                        stringResource(R.string.message_image_save),
                        onSave,
                        tint,
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
