package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.zIndex
import app.wonton.client.TabEnvironment
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 网页层、状态浮层和页面栏；来源页和本机页各用自己的一套代理与凭据。 */
@Composable
internal fun PageLayers(
    state: WebPageState, item: TabItem, page: TabPage, tabs: TabsState, connection: SharedPageConnection,
    plugin: InstalledPlugin?, appearance: AppearanceViewModel, environment: TabEnvironment, viewport: TabViewport,
    supported: Boolean, crossOriginBlocked: Boolean, active: Boolean, overlay: Boolean, cover: Boolean,
    takeover: PageTakeover?, bannerTop: Dp, setTop: (Dp) -> Unit, onDetails: (String) -> Unit,
    onDraft: (String) -> Unit, openLink: (String) -> Unit, back: () -> Unit,
) {
    val script = remember(page.url, page.fullscreen, page.display, appearance, environment) {
        state.script(environment, document = true)
    }
    Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize().imePadding()) {
        if (active) Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()))
        val canLoad = state.started && supported && script != null
        if (canLoad && connection.authorized) {
            PageWebView(state, page, plugin, connection.proxy, environment, requireNotNull(script), viewport.inset,
                active && !cover, openLink, onDraft, connection.profile)
        }
        if (active && page.fullscreen) state.controller.customView?.let { video ->
            AndroidView(factory = { video }, modifier = Modifier.padding(viewport.inset).fillMaxSize())
        }
        if (active) {
            PageOverlays(state, item, page, tabs, connection, supported, crossOriginBlocked, cover,
                onDetails, viewport, bannerTop, setTop, overlay, takeover, back)
        }
    }
}

@Composable
private fun PageOverlays(
    state: WebPageState, item: TabItem, page: TabPage, tabs: TabsState, connection: SharedPageConnection,
    supported: Boolean, crossOriginBlocked: Boolean, cover: Boolean, onDetails: (String) -> Unit,
    viewport: TabViewport, bannerTop: Dp, setTop: (Dp) -> Unit, overlay: Boolean, takeover: PageTakeover?,
    back: () -> Unit,
) {
    val inset = PaddingValues(top = viewport.chromeTop, bottom = viewport.bottom)
    val shared = connection.shared
    if (shared != null) {
        SharedPageStatus(page, shared, connection.source, tabs, state.problem, supported, cover, inset)
    } else {
        PageStatusOverlay(state, page, item.tab, supported, crossOriginBlocked, inset, onDetails)
    }
    PageChrome(item.tab, page, tabs, state.capsule, state.controller, viewport.safe, bannerTop,
        top = setTop, overlay, onBack = back, takeover, onDetails)
}

@Composable
private fun PageChrome(
    tab: TabRef, page: TabPage, tabs: TabsState, capsule: TabCapsuleVisibility,
    controller: PluginWebViewController, safe: PaddingValues, bannerTop: Dp, top: (Dp) -> Unit,
    overlay: Boolean, onBack: () -> Unit, takeover: PageTakeover?, onDetails: (String) -> Unit,
) {
    if (page.fullscreen) {
        PluginPageChrome(tab, page, tabs, safe, bannerTop, shortStatus(page), capsule, controller, takeover,
            onClose = { controller.closeCustomView(); tabs.send(Action.ExitFullscreen(tab)) },
            onReload = { tabs.send(Action.ReloadTab(PlacedTab.Bar(tab))) }, onDetails = onDetails)
    } else {
        val density = LocalDensity.current
        Column(Modifier.onSizeChanged { top(with(density) { it.height.toDp() }) }) {
            PluginTabBar(page, controller.canGoBack, overlay, onBack,
                { tabs.send(Action.CloseTabOverlay) }, takeover,
                onExpand = { tabs.send(Action.EnterFullscreen(tab)) },
                onReload = { tabs.send(Action.ReloadTab(PlacedTab.Bar(tab))) })
            takeover?.let { TakeoverNotice(it) }
        }
    }
}

