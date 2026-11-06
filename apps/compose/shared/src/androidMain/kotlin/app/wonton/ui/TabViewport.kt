package app.wonton.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import app.wonton.protocol.TabLayout
import app.wonton.protocol.TabPage
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Dp
import app.wonton.client.TabEnvironment
import app.wonton.client.TabPlatform
import app.wonton.client.TabSafeArea
import app.wonton.ui.design.WontonTheme
import java.util.TimeZone

@Composable
internal fun tabSafeInsets(): PaddingValues =
    WindowInsets.systemBars.union(WindowInsets.displayCutout).asPaddingValues()

@Composable
internal fun tabEnvironment(top: Dp, bottom: Dp, edge: Boolean, safe: PaddingValues): TabEnvironment {
    val direction = LocalLayoutDirection.current
    return TabEnvironment(
        appLanguageTag(), TimeZone.getDefault().id, TabPlatform.ANDROID,
        WontonTheme.isDark, top.value.toDouble(), bottom.value.toDouble(),
        TabSafeArea(
            if (edge) safe.calculateTopPadding().value.toDouble() else 0.0,
            if (edge) safe.calculateRightPadding(direction).value.toDouble() else 0.0,
            if (edge) safe.calculateBottomPadding().value.toDouble() else 0.0,
            if (edge) safe.calculateLeftPadding(direction).value.toDouble() else 0.0,
        ),
    )
}

internal class TabViewport(
    val safe: PaddingValues, val inset: PaddingValues, val chromeTop: Dp,
    val bottom: Dp, val environment: TabEnvironment,
)

@Composable
internal fun tabViewport(page: TabPage, overlay: Boolean, active: Boolean, top: Dp, bottom: Dp): TabViewport {
    val safe = tabSafeInsets()
    val direction = LocalLayoutDirection.current
    val edge = page.layout == TabLayout.EDGE_TO_EDGE && (!overlay || page.fullscreen)
    val keyboard = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var retainedBottom by remember { mutableStateOf(bottom) }
    SideEffect {
        if (active) retainedBottom = when {
            keyboard -> 0.dp
            page.fullscreen || overlay -> safe.calculateBottomPadding()
            else -> bottom
        }
    }
    // 全屏没有标题栏:网页顶边 = 状态栏下沿(横屏状态栏隐藏,就是 0)。带栏 = 标题栏下沿,胶囊不算在内。
    val chromeTop = if (page.fullscreen) safe.calculateTopPadding() else maxOf(top, safe.calculateTopPadding())
    val inset = if (edge) PaddingValues(0.dp) else PaddingValues(
        start = safe.calculateStartPadding(direction), end = safe.calculateEndPadding(direction),
        top = chromeTop, bottom = retainedBottom,
    )
    return TabViewport(safe, inset, chromeTop, retainedBottom, tabEnvironment(chromeTop, retainedBottom, edge, safe))
}
