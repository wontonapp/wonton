package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.TabPage

/**
 * 一页的层：网页铺满整屏（状态栏、导航条、刘海底下都是网页，安全区经环境交给页面自己让），
 * 上面只浮胶囊和状态浮层。来源页和本机页各用自己的一套代理与凭据。
 * Bot 页没放行或还在等登录确认时不建网页（[admitted]），改由 [disclosure] 说明。
 */
@Composable
internal fun PageLayers(
    state: WebPageState, page: TabPage, pages: PagesState, connection: SharedPageConnection,
    appearance: AppearanceViewModel, viewport: TabViewport, active: Boolean, takeover: PageTakeover?,
    bannerTop: Dp, onDetails: (String) -> Unit, onDraft: (String) -> Unit, openLink: (String) -> Unit,
    admitted: Boolean = true, disclosure: @Composable () -> Unit = {},
) {
    val environment = viewport.environment
    val script = remember(page.url, appearance, environment) { state.script(environment, document = true) }
    Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize().imePadding()) {
        val loadable = connection.supported && connection.authorized && admitted
        if (state.started && loadable && script != null) {
            // 状态栏区域的点击被系统吃掉（#379），网页从状态栏下沿排；胶囊和状态浮层不受影响，
            // 仍按系统安全区排在网页上面。视频全屏（customView）铺满整屏，不吃这条内边距。
            PageWebView(state, page, connection.proxy, environment, script, active && !state.covers(page),
                openLink, onDraft, connection.profile,
                modifier = Modifier.windowInsetsPadding(
                    WindowInsets.systemBars.union(WindowInsets.displayCutout)
                        .only(WindowInsetsSides.Top)),
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
        takeover,
        onClose = { state.controller.closeCustomView(); pages.send(Action.ClosePage) },
        onReload = { pages.send(Action.ReloadTab(state.place)) }, onDetails = onDetails)
}
