package app.wonton.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.MessageContent
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** 到头还往那个方向拖时，位移按这个比例跟手（橡皮筋）。 */
private const val EDGE_RESISTANCE = 0.5f

/** 拉过这么宽的一屏（比例）就算翻页，不用等松手的速度。 */
private const val PAGE_COMMIT_RATIO = 0.25f

/**
 * 松手后翻到哪张：拉过四分之一屏，或朝那个方向轻弹（速度够快）就翻到邻页；否则原地不动。
 * 到头时邻页不存在，结果夹回当前这张（回弹）。
 */
internal fun pageTarget(index: Int, count: Int, drag: Float, velocityX: Float, width: Float, fling: Float): Int = when {
    drag > 0f && (drag > width * PAGE_COMMIT_RATIO || velocityX > fling) -> (index - 1).coerceAtLeast(0)
    drag < 0f && (-drag > width * PAGE_COMMIT_RATIO || velocityX < -fling) -> (index + 1).coerceAtMost(count - 1)
    else -> index
}

/** 超过这个横向速度（dp/s）就算轻弹，距离不够也翻页。 */
internal val PAGE_FLING = 400.dp

/**
 * 同一组照片（一次发送，ADR-0094）的翻页状态：跟手横移、松手按距离或速度翻页，否则弹回原地；
 * 到头没有邻页就回弹，不循环。单位都是像素。
 */
internal class PageState(
    val count: Int,
    startIndex: Int,
    /** 视口宽度，翻页一整页的距离。 */
    private val widthPx: () -> Float,
    private val flingPx: Float,
    private val scope: CoroutineScope,
) {
    var index by mutableIntStateOf(startIndex)
        private set

    /** 跟手横移：正值是往右拉（看前一张），负值往左（看后一张）。 */
    var drag by mutableFloatStateOf(0f)
        private set
    private var settleJob: Job? = null

    /** 方向：-1 往右拉（前一张），1 往左拉（后一张），0 没在拖。 */
    val side: Int
        get() = when {
            drag > 0f -> -1
            drag < 0f -> 1
            else -> 0
        }

    /** 拖动中要不要画邻页：到头时没有邻页，只有当前页的橡皮筋位移。 */
    val neighborIndex: Int?
        get() = (index + side).takeIf { side != 0 && drag != 0f && it in 0 until count }

    /** 手势里每来一段横移调一次；开始新的拖动会取消还没放完的翻页动画，从画到的位置接着跟。 */
    fun applyDrag(dx: Float) {
        settleJob?.cancel()
        // 到头方向：0 是没到头。朝到头再挖的那一段按橡皮筋减速，反手往回的一段照常全速。
        val edge = when (index) {
            0 -> 1f
            count - 1 -> -1f
            else -> 0f
        }
        val intoEdge = edge != 0f && dx * edge > 0f && (drag + dx) * edge > 0f
        drag += if (intoEdge) dx * EDGE_RESISTANCE else dx
    }

    /** 松手：超过四分之一屏或轻弹就翻到邻页，否则（含到头）弹回。 */
    fun settle(velocityX: Float) {
        animateTo(pageTarget(index, count, drag, velocityX, widthPx(), flingPx))
    }

    /** 点左右边区换一张；没有邻页（到头）就不动。 */
    fun tap(side: Int) {
        (index + side).takeIf { it in 0 until count }?.let(::animateTo)
    }

    private fun animateTo(target: Int) {
        settleJob?.cancel()
        val from = drag
        val to = (index - target).toFloat() * widthPx()
        settleJob = scope.launch {
            Animatable(from).animateTo(to, spring(stiffness = Spring.StiffnessMediumLow)) { drag = value }
            if (target != index) index = target
            drag = 0f
        }
    }
}

/**
 * 同一次发送（ADR-0094）的多张照片：按（发送者、组 id）收集、组内按 index 排好，查看器在同组里翻页用。
 * 组 id 只有 32 位，和发送者一起才是 ADR 说的那个组；单张的组不进。
 */
internal fun List<TimelineItem>.imageGroups(): Map<Pair<String, String>, List<ImageContent>> = asSequence()
    .mapNotNull { item -> (item.content as? TimelineContent.Message)?.message }
    .mapNotNull { message ->
        val group = message.group ?: return@mapNotNull null
        val image = (message.body as? MessageContent.Image)?.image ?: return@mapNotNull null
        if (group.count <= 1u) null else (message.senderId to group.id) to (group.index to image)
    }
    .groupBy({ (key, _) -> key }, { (_, pair) -> pair })
    .mapValues { (_, entries) -> entries.sortedBy { it.first }.map { it.second } }

/**
 * 翻页状态：图片或密度变了才重建，起头停在原来那张（那张被删了就停原位）。重建后查看器把新位置写回 [viewing]。
 */
@Composable
internal fun rememberPager(
    images: List<ImageContent>,
    viewing: Pair<MediaHandle, Int>?,
    fallback: Int,
    transform: ViewerTransform,
    density: Density,
    scope: CoroutineScope,
): PageState? = remember(images, density) {
    images.takeIf { it.size > 1 }?.let { group ->
        PageState(
            group.size,
            group.indexOfFirst { image -> image.media == viewing?.first }
                .takeIf { at -> at >= 0 }
                ?: (viewing?.second ?: fallback).coerceIn(0, group.size - 1),
            { transform.viewport.width.toFloat() },
            with(density) { PAGE_FLING.toPx() },
            scope,
        )
    }
}
