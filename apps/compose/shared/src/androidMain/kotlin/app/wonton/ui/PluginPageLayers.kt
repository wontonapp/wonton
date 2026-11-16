package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.zIndex
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef

/**
 * 一页的层：网页从状态栏和胶囊那一行下面铺到屏幕底（导航条底下也是网页，底部安全区经环境交给页面自己让），
 * 上面只浮胶囊和状态浮层。来源页和本机页各用自己的一套代理与凭据。
 * Bot 页没放行或还在等登录确认时不建网页（[admitted]），改由 [disclosure] 说明。
 */
@Composable
internal fun PageLayers(
    state: WebPageState, page: TabPage, pages: PagesState, connection: SharedPageConnection,
    appearance: AppearanceViewModel, viewport: TabViewport, active: Boolean, parked: Boolean,
    takeover: PageTakeover?,
    bannerTop: Dp, onDetails: (String) -> Unit, onDraft: (String) -> Unit, openLink: (String) -> Unit,
    admitted: Boolean = true, disclosure: @Composable () -> Unit = {},
) {
    val environment = viewport.environment
    val script = remember(page.url, appearance, environment, parked) {
        if (parked) state.documentScript else state.script(environment, document = true)
    }
    Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize().imePadding()) {
        // Bot 页面还需先通过 admission。
        val loadable = connection.supported && connection.authorized && admitted
        if (state.started && loadable && script != null) {
            if (state.tab is TabRef.Widget) WidgetPageEffects(state, page, pages)
            // 状态栏区域的点击被系统吃掉（#379），胶囊又会压住页面固定在右上角的按钮：网页从状态栏和胶囊那一行
            // 下面排（viewport.webTop）。状态栏那一截用窗口内边距垫，WebView 就不再把它当安全区报给页面。
            // 视频全屏（customView）铺满整屏，不吃这条内边距。让出来的这一条涂页面自己的颜色（[PageTopStrip]）。
            PageTopStrip(state.controller.pageColor.takeIf { active && !state.covers(page) }, viewport.webTop)
            PageWebView(state, page, connection.proxy, environment, script, active && !state.covers(page),
                openLink, onDraft, connection.profile,
                modifier = Modifier.windowInsetsPadding(
                    WindowInsets.systemBars.union(WindowInsets.displayCutout)
                        .only(WindowInsetsSides.Top))
                    .padding(top = viewport.webTop - viewport.chromeTop),
            )
        }
        disclosure()
        if (active) {
            state.controller.customView?.let { video ->
                AndroidView(factory = { video }, modifier = Modifier.fillMaxSize())
            }
            PageOverlays(state, page, pages, connection, viewport, bannerTop, takeover, onDetails)
        }
    }
}

@Composable
private fun PageOverlays(
    state: WebPageState, page: TabPage, pages: PagesState, connection: SharedPageConnection,
    viewport: TabViewport, bannerTop: Dp, takeover: PageTakeover?, onDetails: (String) -> Unit,
) {
    val inset = PaddingValues(top = viewport.chromeTop, bottom = viewport.bottom)
    val shared = connection.shared
    if (shared != null) {
        SharedPageStatus(page, shared, connection.source, pages, state.problem, connection.supported,
            state.covers(page), inset)
    } else {
        PageStatusOverlay(state, page, state.tab, connection.supported, page.opensInBrowser, inset, onDetails)
    }
    PluginPageChrome(state.tab, pages, viewport.safe, bannerTop, shortStatus(page), state.capsule, state.controller,
        takeover, hint = callRelayHint(state.tab, pages),
        onClose = { state.controller.closeCustomView(); pages.send(Action.Pages(PageAction.ClosePage)) },
        onReload = { pages.send(Action.Pages(PageAction.ReloadTab(state.place))) }, onDetails = onDetails)
}
