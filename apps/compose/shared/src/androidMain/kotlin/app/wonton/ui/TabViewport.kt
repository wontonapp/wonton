package app.wonton.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
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

/** [edge] 为假时（面板里按 Inset 排版的页）网页已经让开了系统栏，安全区报 0。 */
@Composable
internal fun tabEnvironment(top: Dp, bottom: Dp, safe: PaddingValues, edge: Boolean = true): TabEnvironment {
    val direction = LocalLayoutDirection.current
    fun side(value: Dp): Double = if (edge) value.value.toDouble() else 0.0
    return TabEnvironment(
        appLanguageTag(), TimeZone.getDefault().id, TabPlatform.ANDROID,
        WontonTheme.isDark, top.value.toDouble(), bottom.value.toDouble(),
        TabSafeArea(
            side(safe.calculateTopPadding()),
            side(safe.calculateRightPadding(direction)),
            side(safe.calculateBottomPadding()),
            side(safe.calculateLeftPadding(direction)),
        ),
    )
}

internal class TabViewport(
    val safe: PaddingValues, val chromeTop: Dp, val bottom: Dp, val environment: TabEnvironment,
)

/**
 * 整页:网页铺满整屏不加内边距,系统栏和刘海的留白由页面自己按环境里的 safe area 让;胶囊和状态浮层排在安全区下。
 * 键盘弹出时底部安全区报 0(键盘盖住了导航条);离开栈顶的页保留最后一次的值,不跟着别的页重排。
 */
@Composable
internal fun tabViewport(active: Boolean): TabViewport {
    val safe = tabSafeInsets()
    val keyboard = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var retainedBottom by remember { mutableStateOf(safe.calculateBottomPadding()) }
    SideEffect {
        if (active) retainedBottom = if (keyboard) 0.dp else safe.calculateBottomPadding()
    }
    val chromeTop = safe.calculateTopPadding()
    return TabViewport(safe, chromeTop, retainedBottom,
        tabEnvironment(chromeTop, retainedBottom, safe))
}
