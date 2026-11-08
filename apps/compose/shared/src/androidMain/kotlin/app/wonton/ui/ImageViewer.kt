package app.wonton.ui

import android.content.Context
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
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
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

private const val MAX_SCALE = 5f
private const val DOUBLE_TAP_SCALE = 2.5f
private val CLOSE_DRAG_THRESHOLD = 120.dp
/** 双击两下的距离容差：触摸 slop 的四倍，跟系统双击的判定一个量级。 */
private const val DOUBLE_TAP_SLOP_IN_TOUCH_SLOP = 4f
private const val ZOOM_MS = 220
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
    LaunchedEffect(hint) { if (hint != 0) { delay(HINT_MS); hint = 0 } }

    val save: () -> Unit = {
        if (!saving) {
            saving = true
            scope.launch {
                hint = saveToGallery(context, core, image)
                saving = false
            }
        }
    }
    val share: () -> Unit = {
        scope.launch { hint = shareWithOthers(context, core, image) }
    }

    val failed = loaded != null && loaded.isFailure
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
            loaded.isSuccess -> OriginalImage(loaded.getOrThrow(), transform, onDismiss)
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
}

/** 存进相册，返回成功或失败的轻提示文字资源。失败原因在轻提示里放不下，人话按结果说，异常细节丢弃。 */
@Suppress("TooGenericExceptionCaught", "SwallowedException")
private suspend fun saveToGallery(context: Context, core: ClientCore, image: ImageContent): Int = try {
    withContext(Dispatchers.IO) {
        val bytes = core.loadMedia(image.media)
        saveImageToGallery(context, bytes, image.filename, image.mimeType)
    }
    R.string.message_image_saved
} catch (failure: Exception) {
    R.string.message_image_save_failed
}

/** 把图片交给系统的分享面板，返回 0（不打提示）或失败的轻提示文字资源。 */
@Suppress("TooGenericExceptionCaught", "SwallowedException")
private suspend fun shareWithOthers(context: Context, core: ClientCore, image: ImageContent): Int = try {
    val bytes = withContext(Dispatchers.IO) { core.loadMedia(image.media) }
    // 写缓存走 IO，拉分享面板回主线程。
    val (path, mime) = withContext(Dispatchers.IO) {
        shareImageFile(context, bytes, image.filename, image.mimeType)
    }
    shareWithSystem(context, path, mime)
    0
} catch (failure: Exception) {
    R.string.message_image_share_failed
}

@Composable
private fun LoadingOriginal() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(color = Color.White)
    }
}

@Composable
private fun OriginalImage(bitmap: ImageBitmap, transform: ViewerTransform, onDismiss: () -> Unit) {
    Image(
        bitmap,
        contentDescription = null,
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

/**
 * 查看器的缩放与平移。手势循环里（受限挂起的作用域）只能同步改值，所以状态用普通 state，
 * 动画另起协程，用一个进度 Animatable 驱动，保证被按住的点整段动画都跟着手指。
 */
private class ViewerTransform(
    private val scope: CoroutineScope,
    private val closeThresholdPx: Float,
) {
    var scale by mutableFloatStateOf(1f)
        private set
    var offset by mutableStateOf(Offset.Zero)
        private set
    var dismiss by mutableStateOf(Offset.Zero)
        private set
    var viewport = IntSize.Zero
    private var zoomJob: Job? = null
    private var dismissJob: Job? = null

    /** 双指缩放：被捏住的点在屏幕上不动，再叠上双指的平移。 */
    fun applyPinch(event: PointerEvent) {
        val zoomChange = event.calculateZoom()
        val panChange = event.calculatePan()
        if (zoomChange == 1f && panChange == Offset.Zero) return
        val newScale = (scale * zoomChange).coerceIn(1f, MAX_SCALE)
        // O' = c + (O - c) · (新比例 / 旧比例)：被捏住的点跟着手指走。
        val centroid = event.calculateCentroid()
        val target = (offset - centroid) * (newScale / scale) + centroid + panChange
        setZoom(newScale, clamped(target, newScale))
    }

    /** 放大后单指拖动：平移，不超出图片边缘。 */
    fun applyPan(event: PointerEvent) {
        val panChange = event.calculatePan()
        if (panChange == Offset.Zero) return
        setZoom(scale, clamped(offset + panChange, scale))
    }

    /** 跟手下拉：位移直接跟手指。 */
    fun applyDismiss(drag: Offset) {
        dismissJob?.cancel()
        dismiss = drag
    }

    /** 双击：没放大就放大到 [DOUBLE_TAP_SCALE]（点击的点不动），放大了就还原。 */
    fun toggleZoom(centroid: Offset) {
        val targetScale = if (scale > 1f) 1f else DOUBLE_TAP_SCALE
        val targetOffset =
            if (targetScale == 1f) Offset.Zero else clamped(centroid * (1f - targetScale), targetScale)
        animateZoom(targetScale, targetOffset)
    }

    /** 双指缩放结束时比例回到了 1：位置回正。 */
    fun settle() = animateZoom(1f, Offset.Zero)

    /** 没放大时的拖动结束：滑得够远就关掉，否则弹回原位。 */
    fun settleDismiss(dismissed: Offset, onDismiss: () -> Unit) {
        if (dismissed.y > closeThresholdPx) {
            onDismiss()
        } else {
            dismissJob?.cancel()
            val from = dismiss
            dismissJob = scope.launch {
                Animatable(0f).animateTo(1f, spring(stiffness = Spring.StiffnessMediumLow)) {
                    dismiss = lerp(from, Offset.Zero, value)
                }
            }
        }
    }

    private fun setZoom(newScale: Float, newOffset: Offset) {
        zoomJob?.cancel()
        scale = newScale
        offset = newOffset
    }

    /** 缩放动画：一个进度 Animatable 同时驱动比例和位移，按住的点整段动画都不挪。 */
    private fun animateZoom(targetScale: Float, targetOffset: Offset) {
        zoomJob?.cancel()
        val fromScale = scale
        val fromOffset = offset
        zoomJob = scope.launch {
            Animatable(0f).animateTo(1f, tween(ZOOM_MS)) {
                scale = lerp(fromScale, targetScale, value)
                offset = lerp(fromOffset, targetOffset, value)
            }
        }
    }

    /** 平移的边界：比例 1 时图片铺满视口，放大后每边最多让出 (比例 - 1) · 视口 / 2。 */
    private fun clamped(target: Offset, scaleValue: Float): Offset {
        val bound = Offset(
            (scaleValue - 1f) * viewport.width / 2f,
            (scaleValue - 1f) * viewport.height / 2f,
        )
        return Offset(target.x.coerceIn(-bound.x, bound.x), target.y.coerceIn(-bound.y, bound.y))
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
