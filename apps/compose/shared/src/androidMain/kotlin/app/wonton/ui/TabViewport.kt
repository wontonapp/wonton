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

/** [webTop] 是整页网页的顶边到屏幕顶的距离：状态栏加胶囊那一行（[pageCapsuleRow]）。 */
internal class TabViewport(
    val safe: PaddingValues, val chromeTop: Dp, val webTop: Dp, val bottom: Dp, val environment: TabEnvironment,
)

/**
 * 胶囊那一行占的高度（上下各留一格）。整页的网页从这一行下面开始排：胶囊浮在网页右上角时，
 * 现成应用固定在右上角的按钮（Immich 的头像、Actual 的菜单）压在它底下点不到（ADR-0095“铺满”）。
 * Android 的 WebView 没有“这一条被挡住了”的接口（iOS 26 的 `obscuredContentInsets`），只能由网页框让。
 */
@Composable
internal fun pageCapsuleRow(): Dp = WontonTheme.size.smallButton + WontonTheme.spacing.s8 * 2

/**
 * 整页:网页从状态栏和胶囊那一行下面开始排(状态栏区域的点击被系统吃掉,页面够不着,见 [pageEnvironment];
 * 胶囊那一行见 [pageCapsuleRow]);胶囊和状态浮层仍排在系统安全区下。键盘弹出时底部安全区报 0(键盘盖住了导航条);
 * 离开栈顶的页保留最后一次的值,不跟着别的页重排。
 */
@Composable
internal fun tabViewport(active: Boolean, safe: PaddingValues = tabSafeInsets()): TabViewport {
    val keyboard = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var retainedBottom by remember { mutableStateOf(safe.calculateBottomPadding()) }
    SideEffect {
        if (active) retainedBottom = if (keyboard) 0.dp else safe.calculateBottomPadding()
    }
    val chromeTop = safe.calculateTopPadding()
    return TabViewport(
        safe, chromeTop, chromeTop + pageCapsuleRow(), retainedBottom, pageEnvironment(safe, retainedBottom),
    )
}

/**
 * 整页网页拿到的环境:顶部安全区报 0。Android 的状态栏窗口把顶部一条的点击整个吃掉(ADR-0095 的
 * “Android 边到边”在触摸上行不通,#379 实测 Immich/Navidrome 顶栏点不到),网页只能从状态栏下面排;
 * 顶部安全区照实给的话,按桥适配过安全区的页面会自己再让一次,空出两倍。Home 条还在网页下面,底部照实给。
 */
@Composable
private fun pageEnvironment(safe: PaddingValues, bottom: Dp): TabEnvironment {
    val direction = LocalLayoutDirection.current
    return tabEnvironment(0.dp, bottom, PaddingValues(
        top = 0.dp,
        start = safe.calculateLeftPadding(direction),
        end = safe.calculateRightPadding(direction),
        bottom = safe.calculateBottomPadding(),
    ))
}
