package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Total bottom obstruction, supplied by the native shell without resizing its viewport. */
val LocalWontonBottomInset = compositionLocalOf { 0.dp }

/** Pinned bar's measured height (status bar included). */
val LocalWontonTopInset = compositionLocalOf { 0.dp }

/** Apply inside verticalScroll, so these clearances scroll away rather than clipping the viewport. */
@Composable
fun Modifier.wontonContentPadding(): Modifier = padding(wontonContentPaddingValues())

@Composable
fun wontonContentPaddingValues(
    horizontal: Dp = 0.dp,
    top: Dp = 0.dp,
    bottom: Dp = 0.dp,
): PaddingValues = PaddingValues(
    start = horizontal,
    top = LocalWontonTopInset.current + top,
    end = horizontal,
    bottom = LocalWontonBottomInset.current + bottom,
)

/**
 * Full-screen content beneath a pinned Material bar; only scroll content receives clearance.
 *
 * 栏高在这一次测量里定下来：先量栏，再用实测高度 composition-local 给内容、量内容。
 * 用状态回填栏高时，内容先按 0 让位，要到下一帧才补上；一帧定格的快照会抽到栏压内容的帧。
 */
@Composable
fun WontonScreen(bar: @Composable () -> Unit, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    val canvas = WontonTheme.colors.canvas.toColor()
    // 向上滚第一像素内容就进了顶栏下面；回到顶（剩余向下分量没人消费）时恢复透明。
    // 不用 pinnedScrollBehavior：小顶栏的 overlappedFraction 恒为 0，collapsedFraction 也不随 nested scroll 变化。
    var scrolledUnder by remember { mutableStateOf(false) }
    val overlap = remember {
        object : NestedScrollConnection {
            override fun onPreScroll(available: Offset, source: NestedScrollSource) =
                if (available.y < 0f) {
                    scrolledUnder = true
                    Offset.Zero
                } else {
                    Offset.Zero
                }

            override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource) =
                if (available.y > 0f) {
                    scrolledUnder = false
                    Offset.Zero
                } else {
                    Offset.Zero
                }
        }
    }
    val fill = if (scrolledUnder) canvas.copy(alpha = 0.85f) else Color.Transparent
    val navigationInset = with(density) { WindowInsets.navigationBars.getBottom(this).toDp() }
    val bottomInset = maxOf(LocalWontonBottomInset.current, navigationInset)
    SubcomposeLayout(modifier.fillMaxSize().background(canvas).nestedScroll(overlap)) { constraints ->
        val loose = constraints.copy(minWidth = 0, minHeight = 0)
        val barPlaceables = subcompose(BarSlot) { Box(Modifier.background(fill)) { bar() } }
            .map { it.measure(loose) }
        val barHeightPx = barPlaceables.maxOfOrNull { it.height } ?: 0
        val contentPlaceables = subcompose(ContentSlot) {
            CompositionLocalProvider(
                LocalWontonTopInset provides with(density) { barHeightPx.toDp() },
                LocalWontonBottomInset provides bottomInset,
            ) { content() }
        }.map { it.measure(loose) }
        val width = if (constraints.hasBoundedWidth) constraints.maxWidth
        else maxOf(contentPlaceables.maxOfOrNull { it.width } ?: 0, barPlaceables.maxOfOrNull { it.width } ?: 0)
        val height = if (constraints.hasBoundedHeight) constraints.maxHeight
        else maxOf(contentPlaceables.maxOfOrNull { it.height } ?: 0, barHeightPx)
        layout(width, height) {
            contentPlaceables.forEach { it.place(0, 0) }
            barPlaceables.forEach { it.place(0, 0) }
        }
    }
}

private object BarSlot
private object ContentSlot
