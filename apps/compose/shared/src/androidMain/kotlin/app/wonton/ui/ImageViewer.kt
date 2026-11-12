package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.lerp
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.util.lerp
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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

private val CLOSE_DRAG_THRESHOLD = 120.dp
private const val HINT_MS = 2_500L

/**
 * 图片的全屏查看器：原图、双指缩放、双击放大/还原、放大后拖动、下滑关闭；顶栏存进相册和分享。
 * 原图还在下载时转圈，取不到时给出可重试的说明。
 */
@Composable
internal fun ImageViewer(image: ImageContent, core: ClientCore, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        ImageViewerContent(image, core, onDismiss)
    }
}

@Composable
private fun ImageViewerContent(image: ImageContent, core: ClientCore, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val transform = remember(scope, density) {
        ViewerTransform(scope, with(density) { CLOSE_DRAG_THRESHOLD.toPx() })
    }
    var attempt by remember { mutableIntStateOf(0) }
    val loaded = loadedImage(core, image.media, attempt).value
    var saving by remember { mutableStateOf(false) }
    var hint by remember { mutableStateOf(0) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    LaunchedEffect(hint) { if (hint != 0) { delay(HINT_MS); hint = 0 } }

    val save: () -> Unit = {
        if (!saving) {
            saving = true
            scope.launch {
                problem = saveToGallery(context, core, image)
                if (problem == null) hint = R.string.message_image_saved
                saving = false
            }
        }
    }
    val share: () -> Unit = {
        scope.launch { problem = shareWithOthers(context, core, image) }
    }

    val failed = loaded != null && loaded.isFailure
    val photo = stringResource(R.string.message_quote_image)
    Box(
        Modifier.fillMaxSize()
            .onSizeChanged { transform.viewport = it }
            .graphicsLayer {
                // 跟手下拉时整层淡出，松手够远就关。
                alpha = 1f - (transform.dismiss.y / transform.viewport.height.coerceAtLeast(1))
                    .coerceIn(0f, 1f)
            }
            .background(Color.Black),
    ) {
        when {
            loaded == null -> LoadingOriginal()
            loaded.isSuccess -> {
                val label = image.filename.ifEmpty { photo }
                OriginalImage(loaded.getOrThrow(), transform, onDismiss, label)
            }
            else -> OriginalFailed(loaded.exceptionOrNull()) { attempt++ }
        }
        TopBar(
            onClose = onDismiss,
            saving = saving,
            onSave = save,
            onShare = share,
            onBlack = !failed,
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
    null
} catch (failure: CancellationException) {
    throw failure
} catch (failure: Exception) {
    imageOperationProblem(failure, R.string.message_image_share_failed)
}


@Composable
private fun LoadingOriginal() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(color = Color.White)
    }
}

/** 原图：文件名当读屏名字，和 iOS 的查看器一致；没有文件名时用“图片”。 */
@Composable
private fun OriginalImage(bitmap: ImageBitmap, transform: ViewerTransform, onDismiss: () -> Unit, label: String) {
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
private fun OriginalFailed(failure: Throwable?, onRetry: () -> Unit) {
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


/** 查看器的手势：双指缩放、放大后拖动、没放大时跟手下滑，两下没动的点击算双击。 */
private suspend fun PointerInputScope.viewerGestures(transform: ViewerTransform, onDismiss: () -> Unit) {
    awaitEachGesture {
        val firstDown = awaitFirstDown(requireUnconsumed = false)
        if (!trackGesture(transform, onDismiss)) return@awaitEachGesture
        // 第一次是点击：在双击时限内等第二下，两下都没动才算双击。
        val secondDown = withTimeoutOrNull(viewConfiguration.doubleTapTimeoutMillis) {
            awaitFirstDown(requireUnconsumed = false)
        } ?: return@awaitEachGesture
        val slop = viewConfiguration.touchSlop * DOUBLE_TAP_SLOP_IN_TOUCH_SLOP
        val secondWasTap = trackGesture(transform, onDismiss)
        if (secondWasTap && (secondDown.position - firstDown.position).getDistance() <= slop) {
            transform.toggleZoom(secondDown.position)
        }
    }
}

/** 一次手势从按下到抬起：跟手缩放、平移或下滑。返回它是不是一次点击（没动过）。 */
private suspend fun AwaitPointerEventScope.trackGesture(
    transform: ViewerTransform,
    onDismiss: () -> Unit,
): Boolean {
    var zooming = false
    var tap = true
    var dismissed = Offset.Zero
    while (true) {
        val event = awaitPointerEvent()
        val pressed = event.changes.filter { it.pressed }
        if (pressed.isEmpty()) break
        when {
            pressed.size >= 2 -> {
                if (!zooming) {
                    zooming = true
                    tap = false
                    dismissed = Offset.Zero
                    transform.applyDismiss(Offset.Zero)
                }
                transform.applyPinch(event)
            }
            transform.scale > 1f -> {
                // 放大状态下没动过的点击仍是 tap，双击还原才认得出来。
                val drag = event.calculatePan()
                if (drag != Offset.Zero) tap = false
                transform.applyPan(event)
            }
            else -> {
                val drag = event.calculatePan()
                if (drag != Offset.Zero) tap = false
                dismissed += drag
                transform.applyDismiss(dismissed)
            }
        }
        event.changes.forEach { it.consume() }
    }
    if (zooming && transform.scale <= 1f) transform.settle()
    if (!zooming) transform.settleDismiss(dismissed, onDismiss)
    return tap
}
