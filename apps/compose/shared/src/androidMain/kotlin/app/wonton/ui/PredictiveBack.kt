package app.wonton.ui

import androidx.activity.BackEventCompat
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.util.lerp
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** 拖到底时上面一页缩到多大（Material 的返回预览是 90%）。 */
private const val LEAVING_SCALE = 0.9f

/** 上面一页往手指方向挪的距离：屏宽的 1/20 减 8dp（Material 返回预览的口径）。 */
private const val LEAVING_SHIFT_RATIO = 1f / 20f
private val LEAVING_SHIFT_INSET = 8.dp
private val LEAVING_CORNER = 28.dp

/** 下面一页起手时压暗多少，跟 iOS 的 `EdgePopAnimator` 一致，随进度亮起来。 */
private const val UNDER_DIM = 0.1f

/** 系统判定返回以后，上面一页淡出、再往外滑一截的时长和距离。 */
private const val EXIT_MS = 160
private const val EXIT_SLIDE_RATIO = 0.25f

/** 返回经核心换页（关页、收起应用网格）时等多久；超时就收起预览，照实露出核心给的那一页。 */
private const val SETTLE_MS = 1_000L

/**
 * 一次系统返回手势的预览（Android 14 起的 predictive back）：系统给进度、判定完成还是取消，这里只画。
 * 拖动时 [leaving]（上面一页）跟手缩小、挪开、圆角，[under]（返回后露出来的那页）垫在下面先压暗、随进度亮起来；
 * 系统判定返回后上面一页淡出，再交给调用方改导航；取消就弹回。
 */
@Stable
internal class BackPreview<K : Any> {
    var leaving: K? by mutableStateOf(null)
        private set
    var under: K? by mutableStateOf(null)
        private set
    internal var fromRight by mutableStateOf(false)
        private set
    internal val progress = Animatable(0f)
    internal val exit = Animatable(0f)

    internal suspend fun start(top: K, target: K, edge: Int) {
        progress.snapTo(0f)
        exit.snapTo(0f)
        fromRight = edge == BackEventCompat.EDGE_RIGHT
        leaving = top
        under = target
    }

    internal suspend fun cancel() {
        progress.animateTo(0f)
        reset()
    }

    internal suspend fun reset() {
        leaving = null
        under = null
        progress.snapTo(0f)
        exit.snapTo(0f)
    }
}

/**
 * 这一层的返回：[under] 不为 null 时接系统返回，返回后显示 [under]。手势拖动时预览（见 [BackPreview]），
 * 系统判定完成后调 [onBack]；返回键、三键导航和 Android 13 及以下没有进度，直接调 [onBack]、不画动画。
 * [onBack] 可能经核心异步换页：预览等到这一层的顶（[top]）换掉才收起，免得旧页闪回来。
 */
@Composable
internal fun <K : Any> PredictiveBack(preview: BackPreview<K>, top: K, under: K?, onBack: () -> Unit) {
    val currentTop by rememberUpdatedState(top)
    val currentOnBack by rememberUpdatedState(onBack)
    PredictiveBackHandler(enabled = under != null) { events ->
        val target = under ?: return@PredictiveBackHandler
        previewBack(preview, events, currentTop, target, { currentTop }) { currentOnBack() }
    }
}

/** [PredictiveBack] 的手势部分；网页那层自己决定要不要预览，也走这里。 */
internal suspend fun <K : Any> previewBack(
    preview: BackPreview<K>,
    events: kotlinx.coroutines.flow.Flow<BackEventCompat>,
    top: K,
    target: K,
    currentTop: () -> K,
    onBack: () -> Unit,
) {
    var moved = false
    try {
        events.collect { event ->
            if (!moved) {
                moved = true
                preview.start(top, target, event.swipeEdge)
            }
            preview.progress.snapTo(event.progress)
        }
    } catch (cancelled: CancellationException) {
        // 系统判定取消（没拖够、往回拖）时整个协程被取消：弹回要在取消之外走完。
        if (moved) withContext(NonCancellable) { preview.cancel() }
        throw cancelled
    }
    if (!moved) {
        onBack()
        return
    }
    try {
        preview.exit.animateTo(1f, tween(EXIT_MS, easing = FastOutLinearInEasing))
        onBack()
        withTimeoutOrNull(SETTLE_MS) { snapshotFlow { currentTop() }.first { it != top } }
    } finally {
        withContext(NonCancellable) { preview.reset() }
    }
}

/** 上面那页拖动时的样子；只读动画值，不触发重组。 */
internal fun Modifier.backLeaving(preview: BackPreview<*>): Modifier = graphicsLayer {
    val p = preview.progress.value
    val e = preview.exit.value
    val scale = lerp(1f, LEAVING_SCALE, p)
    scaleX = scale
    scaleY = scale
    val shift = (size.width * LEAVING_SHIFT_RATIO - LEAVING_SHIFT_INSET.toPx()).coerceAtLeast(0f) * p +
        size.width * EXIT_SLIDE_RATIO * e
    translationX = if (preview.fromRight) -shift else shift
    alpha = 1f - e
    shape = RoundedCornerShape(LEAVING_CORNER * p)
    clip = true
}

/** 盖在下面那页上的压暗，随进度和淡出亮起来。 */
@Composable
internal fun BackUnderDim(preview: BackPreview<*>, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxSize().drawBehind {
        val dim = UNDER_DIM * (1f - preview.progress.value) * (1f - preview.exit.value)
        drawRect(Color.Black.copy(alpha = dim))
    })
}

/**
 * 一层叠着的原生页：[pages] 从上往下排，第一项是现在显示的页，第二项是返回后显示的页，最后一项是这层的根。
 * 平时只组合最上面那页；返回手势拖动时把第二项也组合出来垫在下面（见 [BackPreview]）。[keyOf] 认页（也用来
 * 存各页的滚动位置），要能存进 Bundle，同一类页用同一个（换内容时原地更新，不重建）；[onBack] 关掉传进来的那页。
 */
@Composable
internal fun <K : Any> BackLayers(
    pages: List<K>,
    keyOf: (K) -> String,
    onBack: (K) -> Unit,
    content: @Composable (K) -> Unit,
) {
    val preview = remember { BackPreview<K>() }
    val holder = rememberSaveableStateHolder()
    // 存着的滚动位置只留给还在这层里的页：关掉的页下次打开从头来，和换页前一样。
    val keys = pages.map(keyOf)
    val kept = remember { mutableSetOf<String>() }
    SideEffect {
        (kept - keys.toSet()).forEach(holder::removeState)
        kept.clear()
        kept.addAll(keys)
    }
    val top = pages.first()
    PredictiveBack(preview, top, pages.getOrNull(1)) { onBack(top) }
    val under = preview.under?.takeIf { keyOf(it) != keyOf(top) }
    Box(Modifier.fillMaxSize()) {
        BackPreviewPages(preview, listOfNotNull(under, top), keyOf) { page ->
            holder.SaveableStateProvider(keyOf(page)) { content(page) }
        }
    }
}

/**
 * 画 [shown]（下面那页在前、上面那页在后）：预览中的上面一页跟手变形，下面那页盖一层压暗。各页按 [keyOf]
 * 排进同一个带 key 的循环，返回完成后下面那页原地变成上面一页，不重建。
 */
@Composable
internal fun <K : Any> BackPreviewPages(
    preview: BackPreview<K>,
    shown: List<K>,
    keyOf: (K) -> String,
    page: @Composable (K) -> Unit,
) {
    val leavingKey = preview.leaving?.let(keyOf)
    val underKey = preview.under?.let(keyOf)
    val canvas = WontonTheme.colors.canvas.toColor()
    for (one in shown) {
        val id = keyOf(one)
        key(id) {
            val leaving = id == leavingKey
            Box(
                if (leaving) Modifier.fillMaxSize().backLeaving(preview).background(canvas)
                else Modifier.fillMaxSize(),
            ) {
                page(one)
                if (id == underKey && !leaving) BackUnderDim(preview)
            }
        }
    }
}

/** 盖在整个主界面上的一页（下面一直画着）：拖返回时它跟手变形、下面压暗，判定返回后调 [onBack] 收掉它。 */
@Composable
internal fun BackOverlay(onBack: () -> Unit, content: @Composable () -> Unit) {
    val preview = remember { BackPreview<Boolean>() }
    PredictiveBack(preview, top = true, under = false, onBack = onBack)
    val leaving = preview.leaving != null
    Box(Modifier.fillMaxSize()) {
        if (leaving) BackUnderDim(preview)
        Box(
            if (leaving) Modifier.fillMaxSize().backLeaving(preview).background(WontonTheme.colors.canvas.toColor())
            else Modifier.fillMaxSize(),
        ) { content() }
    }
}
