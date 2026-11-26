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
 * 栏高在这一次测量里定下来：先量栏，把实测高度写进状态，再给内容、量内容。
 * 状态在 subcompose 内容之前已经更新，内容第一次组合就读到真值，以后高度变了也会让读方重组。
 * （只传一个非状态变量不行：同一个键复用子组合时不会重跑 subcompose 的 lambda，内容会一直拿着首测的 0。）
 */
@Composable
fun WontonScreen(bar: @Composable () -> Unit, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    var barHeight by remember { mutableStateOf(0.dp) }
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
        barHeight = with(density) { barHeightPx.toDp() }
        val contentPlaceables = subcompose(ContentSlot) {
            CompositionLocalProvider(
                LocalWontonTopInset provides barHeight,
                LocalWontonBottomInset provides bottomInset,
            ) { content() }
        }.map { it.measure(loose) }
        val width = if (constraints.hasBoundedWidth) constraints.maxWidth
        else maxOf(contentPlaceables.maxOfOrNull { it.width } ?: 0, barPlaceables.maxOfOrNull { it.width } ?: 0)
        val height = if (constraints.hasBoundedHeight) constraints.maxHeight
        else maxOf(contentPlaceables.maxOfOrNull { it.height } ?: 0, barHeightPx)
        layout(width, height) {
            contentPlaceables.forEach { it.placeRelative(0, 0) }
            barPlaceables.forEach { it.placeRelative(0, 0) }
        }
    }
}

private object BarSlot
private object ContentSlot
