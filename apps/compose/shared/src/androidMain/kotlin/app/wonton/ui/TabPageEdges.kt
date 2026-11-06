package app.wonton.ui

import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.view.PixelCopy
import android.view.View
import android.view.Window
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.core.graphics.ColorUtils
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

private const val ICON_CONTRAST_CROSSOVER = 0.179
private const val EDGE_SAMPLE_INTERVAL_MS = 1_000L

internal enum class TabEdge { TOP, BOTTOM, LEFT, RIGHT }

internal class TabEdgePixels(val image: ImageBitmap, val lightBackground: Boolean)

internal class TabPageEdges {
    val pixels = mutableStateMapOf<TabEdge, TabEdgePixels>()
    private val handler = Handler(Looper.getMainLooper())

    suspend fun sample(window: Window, view: View, sides: Set<TabEdge>, obscuredTop: Int) {
        if (!view.isShown || view.width < 1 || view.height < 1) return
        val location = IntArray(2)
        view.getLocationInWindow(location)
        val bounds = Rect(location[0], location[1], location[0] + view.width, location[1] + view.height)
        for (side in sides) {
            val rect = edgeRect(side, bounds)
            // PixelCopy reads the window, not just the WebView. A full-width confirmation banner
            // can cover the edge-to-edge page's top row; wait rather than sample the banner.
            if (rect.top < obscuredTop) continue
            copy(window, rect)?.let { bitmap ->
                pixels[side] = TabEdgePixels(bitmap.asImageBitmap(), lightBackground(bitmap))
            }
        }
    }

    private suspend fun copy(window: Window, rect: Rect): Bitmap? = suspendCancellableCoroutine { continuation ->
        val bitmap = Bitmap.createBitmap(rect.width(), rect.height(), Bitmap.Config.ARGB_8888)
        try {
            PixelCopy.request(window, rect, bitmap, { result ->
                if (result == PixelCopy.SUCCESS && continuation.isActive) {
                    continuation.resume(bitmap)
                } else {
                    bitmap.recycle()
                    if (continuation.isActive) continuation.resume(null)
                }
            }, handler)
        } catch (_: IllegalArgumentException) {
            bitmap.recycle()
            continuation.resume(null)
        }
    }
}

private fun edgeRect(side: TabEdge, bounds: Rect): Rect = when (side) {
    TabEdge.TOP -> Rect(bounds.left, bounds.top, bounds.right, bounds.top + 1)
    TabEdge.BOTTOM -> Rect(bounds.left, bounds.bottom - 1, bounds.right, bounds.bottom)
    TabEdge.LEFT -> Rect(bounds.left, bounds.top, bounds.left + 1, bounds.bottom)
    TabEdge.RIGHT -> Rect(bounds.right - 1, bounds.top, bounds.right, bounds.bottom)
}

private fun lightBackground(bitmap: Bitmap): Boolean {
    var luminance = 0.0
    for (y in 0 until bitmap.height) for (x in 0 until bitmap.width) {
        luminance += ColorUtils.calculateLuminance(bitmap.getPixel(x, y))
    }
    // Choose the icon color with the higher contrast, rather than assuming the App theme.
    return luminance / (bitmap.width * bitmap.height) > ICON_CONTRAST_CROSSOVER
}

@Composable
internal fun SampleTabEdges(
    edges: TabPageEdges, controller: PluginWebViewController, window: Window?, visible: Boolean,
    gaps: PaddingValues, fullscreen: Boolean, webBottom: Boolean, bannerTop: Dp,
) {
    val density = LocalDensity.current
    val direction = LocalLayoutDirection.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val navigationEdge = tabNavigationEdge()
    val sides = remember(gaps, fullscreen, webBottom, direction, navigationEdge) {
        buildSet {
            if (fullscreen || gaps.calculateTopPadding().value > 0) add(TabEdge.TOP)
            if (gaps.calculateBottomPadding().value > 0) add(TabEdge.BOTTOM)
            if (webBottom) add(navigationEdge)
            if (gaps.calculateLeftPadding(direction).value > 0) add(TabEdge.LEFT)
            if (gaps.calculateRightPadding(direction).value > 0) add(TabEdge.RIGHT)
        }
    }
    val obscuredTop = with(density) { bannerTop.roundToPx() }
    LaunchedEffect(edges, controller, window, visible, sides, obscuredTop, lifecycle) {
        if (!visible || window == null || sides.isEmpty()) return@LaunchedEffect
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (isActive) {
                controller.view?.let { edges.sample(window, it, sides, obscuredTop) }
                delay(EDGE_SAMPLE_INTERVAL_MS)
            }
        }
    }
}

@Composable
internal fun TabPageBackdrop(edges: TabPageEdges, gaps: PaddingValues) {
    val direction = LocalLayoutDirection.current
    BoxWithConstraints(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
        val top = gaps.calculateTopPadding()
        val bottom = gaps.calculateBottomPadding()
        val left = gaps.calculateLeftPadding(direction)
        val right = gaps.calculateRightPadding(direction)
        val middle = (maxHeight - top - bottom).coerceAtLeast(0.dp)
        EdgeStrip(edges, TabEdge.TOP, Modifier.size(maxWidth, top))
        EdgeStrip(edges, TabEdge.BOTTOM, Modifier.offset(y = maxHeight - bottom).size(maxWidth, bottom))
        EdgeStrip(edges, TabEdge.LEFT, Modifier.offset(y = top).size(left, middle))
        EdgeStrip(edges, TabEdge.RIGHT, Modifier.offset(x = maxWidth - right, y = top).size(right, middle))
    }
}

@Composable
private fun EdgeStrip(edges: TabPageEdges, side: TabEdge, modifier: Modifier) {
    Box(modifier) {
        edges.pixels[side]?.let {
            Image(it.image, contentDescription = null, Modifier.fillMaxSize(), contentScale = ContentScale.FillBounds)
        }
    }
}
