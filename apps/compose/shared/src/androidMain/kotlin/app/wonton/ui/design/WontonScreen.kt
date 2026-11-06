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
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Total bottom obstruction, supplied by the native shell without resizing its viewport. */
val LocalWontonBottomInset = compositionLocalOf { 0.dp }
private val LocalWontonTopInset = compositionLocalOf { 0.dp }

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

/** Full-screen content beneath a pinned Material bar; only scroll content receives clearance. */
@Composable
fun WontonScreen(bar: @Composable () -> Unit, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    var barHeight by remember { mutableIntStateOf(0) }
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
    Box(modifier.fillMaxSize().background(canvas).nestedScroll(overlap)) {
        CompositionLocalProvider(
            LocalWontonTopInset provides with(density) { barHeight.toDp() },
            LocalWontonBottomInset provides bottomInset,
        ) { content() }
        Box(Modifier.background(fill).onSizeChanged { barHeight = it.height }) { bar() }
    }
}
