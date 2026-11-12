package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.WebViewFeature
import app.wonton.client.TabBridgeAction
import app.wonton.client.TabEnvironment
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlin.math.roundToInt

@Composable
fun PluginTabsHost(
    tabs: TabsState,
    home: HomeViewModel,
    appearance: AppearanceViewModel,
    bottom: Dp,
    active: TabRef?,
    reselected: TabRef?,
    onReselected: () -> Unit,
    onDetails: (String) -> Unit,
    onDraft: (String) -> Unit,
    bannerTop: Dp,
) {
    val visited = remember { mutableStateMapOf<TabRef, Boolean>() }
    val items = tabs.viewModel.items + listOfNotNull(tabs.viewModel.overlay)
    SideEffect {
        visited.keys.retainAll(items.map { it.tab }.toSet())
        if (active is TabRef.Plugin) visited[active] = true
    }
    items.filter { it.page != null && (it.tab == active || visited[it.tab] == true) }.forEach { item ->
        key(item.tab) {
            PluginTabContainer(
                item, tabs, home, appearance, bottom, active == item.tab,
                overlay = tabs.viewModel.overlay?.tab == item.tab,
                reselected = reselected == item.tab, onReselected, onDetails, onDraft, bannerTop,
            )
        }
    }
}

private class WebPageState(val tab: TabRef, val tabs: TabsState) {
    val controller = PluginWebViewController()
    var loaded by mutableStateOf(false)
    var failed by mutableStateOf(false)
    var started by mutableStateOf(false)
    var problem by mutableStateOf<Problem?>(null)

    fun covers(page: TabPage): Boolean {
        val available = page.state is TabState.Ready || page.state is TabState.Upgrading
        return failed || !loaded || !available
    }

    fun script(environment: TabEnvironment, document: Boolean = false): String? = try {
        if (document) tabs.core.tabDocumentScript(tab, environment) else tabs.core.tabContextScript(tab, environment)
    } catch (failure: CoreException) {
        problem = failure.problem
        null
    }

    fun back(fullscreen: Boolean, overlay: Boolean) {
        if (controller.closeCustomView() || controller.goBack()) return
        if (fullscreen) tabs.send(Action.ExitFullscreen(tab))
        else if (overlay) tabs.send(Action.CloseTabOverlay)
        else tabs.viewModel.items.firstOrNull()?.let { tabs.send(Action.SelectTab(it.tab)) }
    }

    fun loadStarted() { loaded = false; failed = false }

    fun loadFinished(environment: TabEnvironment) {
        loaded = true
        failed = false
        tabs.send(Action.TabLoadSucceeded(tab))
        script(environment)?.let { controller.evaluateJavascript(it) }
        controller.evaluateJavascript(tabs.core.tabConnectionScript())
    }

    fun loadFailed(status: Int?) {
        failed = true
        tabs.send(Action.TabLoadFailed(tab, status?.toUShort()))
    }
}

@Composable
private fun PluginTabContainer(
    item: TabItem, tabs: TabsState, home: HomeViewModel, appearance: AppearanceViewModel,
    bottom: Dp, active: Boolean, overlay: Boolean, reselected: Boolean,
    onReselected: () -> Unit, onDetails: (String) -> Unit, onDraft: (String) -> Unit,
    bannerTop: Dp,
) {
    val plugin = tabs.installedPlugin(item.tab)
    val page = item.page ?: return
    val state = remember { WebPageState(item.tab, tabs) }
    var top by remember { mutableStateOf(0.dp) }
    val viewport = tabViewport(page, overlay, active, top, bottom)
    val edges = remember { TabPageEdges() }
    val environment = viewport.environment
    val script = remember(page.url, page.fullscreen, page.display, appearance, environment) {
        state.script(environment, document = true)
    }
    val openLink = externalLinkOpener { state.problem = it }
    val supported = remember {
        listOf(WebViewFeature.WEB_MESSAGE_LISTENER, WebViewFeature.DOCUMENT_START_SCRIPT,
            WebViewFeature.PROXY_OVERRIDE, WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS)
            .all(WebViewFeature::isFeatureSupported)
    }
    val crossOriginBlocked = isCrossOriginIsolationBlocked(plugin)
    PageEffects(state, page, home, active, appearance, environment)
    ReselectedEffect(state, reselected, onReselected)
    val back = { state.back(page.fullscreen, overlay) }
    TabBackHandler(state, active, page.fullscreen, overlay, back)
    val cover = state.covers(page)
    val takeover = pageTakeover(item.tab, tabs, active)
    PageEdgeEffects(state, page, viewport, edges, active, bannerTop)
    Box(
        Modifier.zIndex(if (active) 1f else 0f).fillMaxSize().imePadding(),
    ) {
        if (active) TabPageBackdrop(edges, viewport.gaps)
        if (state.started && supported && script != null) {
            PageWebView(state, page, plugin, home, environment, script, viewport.inset, active && !cover, openLink,
                onDraft)
        }
        if (active && page.fullscreen) state.controller.customView?.let { video ->
            AndroidView(factory = { video }, modifier = Modifier.padding(viewport.inset).fillMaxSize())
        }
        if (active) {
            PageStatusOverlay(state, page, item.tab, supported, crossOriginBlocked,
                PaddingValues(top = top, bottom = viewport.bottom), onDetails)
            PageChrome(state, item, page, viewport.safe, bannerTop, overlay, back, takeover,
                onDetails, onTop = { top = it })
        }
    }
}

@Composable
private fun PageEdgeEffects(
    state: WebPageState, page: TabPage, viewport: TabViewport, edges: TabPageEdges,
    active: Boolean, bannerTop: Dp,
) {
    val visible = active && !state.covers(page) && state.problem == null && state.controller.customView == null
    SampleTabEdges(edges, state.controller, LocalActivity.current?.window, visible,
        viewport.gaps, page.fullscreen, viewport.webBottom, bannerTop)
    if (active) PageSystemBarIcons(edges, page.fullscreen, viewport.webBottom && !viewport.keyboard)
}

@Composable
private fun PageStatusOverlay(
    state: WebPageState, page: TabPage, tab: TabRef, supported: Boolean, crossOriginBlocked: Boolean,
    inset: PaddingValues, onDetails: (String) -> Unit,
) {
    val platformBlocked = !supported || crossOriginBlocked
    if (state.covers(page) || platformBlocked || state.problem != null) {
        Box(Modifier.padding(inset).fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
            PluginTabStatus(page, tab, state.tabs, state.problem, supported, crossOriginBlocked, onDetails)
        }
    }
}

@Composable
private fun PageChrome(
    state: WebPageState, item: TabItem, page: TabPage, safe: PaddingValues,
    bannerTop: Dp, overlay: Boolean, back: () -> Unit, takeover: PageTakeover?,
    onDetails: (String) -> Unit, onTop: (Dp) -> Unit,
) {
    val density = LocalDensity.current
    val direction = LocalLayoutDirection.current
    Column(Modifier.onSizeChanged { onTop(with(density) { it.height.toDp() }) }
        .padding(top = if (page.fullscreen) bannerTop else 0.dp)) {
        val reload = { state.problem = null; state.tabs.send(Action.ReloadTab(item.tab)) }
        if (page.fullscreen) {
            FullscreenTabBar(item.tab, page, state.tabs,
                PaddingValues(start = safe.calculateStartPadding(direction),
                    top = if (bannerTop > 0.dp) 0.dp else safe.calculateTopPadding(),
                    end = safe.calculateEndPadding(direction)),
                onClose = { state.controller.closeCustomView(); state.tabs.send(Action.ExitFullscreen(item.tab)) },
                onReload = reload, onDetails = onDetails)
        } else {
            PluginTabBar(page, state.controller.canGoBack, overlay, back,
                { state.tabs.send(Action.CloseTabOverlay) }, takeover,
                onExpand = { state.tabs.send(Action.EnterFullscreen(item.tab)) }, onReload = reload)
            takeover?.let { TakeoverNotice(it) }
        }
    }
}

@Composable
private fun ReselectedEffect(state: WebPageState, reselected: Boolean, onHandled: () -> Unit) {
    LaunchedEffect(reselected) {
        if (reselected) {
            state.controller.scrollToTop()
            state.controller.evaluateJavascript(state.tabs.core.tabReselectedScript())
            onHandled()
        }
    }
}

@Composable
private fun TabBackHandler(
    state: WebPageState, active: Boolean, fullscreen: Boolean, overlay: Boolean, back: () -> Unit,
) {
    val canLeave = fullscreen || overlay || state.tabs.viewModel.items.firstOrNull()?.tab != state.tab
    BackHandler(active && (state.controller.customView != null || state.controller.canGoBack || canLeave), back)
}

@Composable
private fun PageEffects(
    state: WebPageState, page: TabPage, home: HomeViewModel, active: Boolean,
    appearance: AppearanceViewModel, environment: TabEnvironment,
) {
    val connected = home.status is HomeStatus.Connected
    val fontScale = LocalDensity.current.fontScale
    LaunchedEffect(active, connected, page.state) {
        if (active && connected && page.state is TabState.Ready) state.started = true
    }
    LaunchedEffect(state.controller.customView, active) {
        if (state.controller.customView != null) {
            if (!active) state.controller.closeCustomView()
            else if (!page.fullscreen) state.tabs.send(Action.EnterFullscreen(state.tab))
        }
    }
    LaunchedEffect(appearance, environment, page.fullscreen, page.display, fontScale) {
        if (state.started) state.script(environment)?.let { state.controller.evaluateJavascript(it) }
    }
    LaunchedEffect(home.status) { state.controller.evaluateJavascript(state.tabs.core.tabConnectionScript()) }
}

@Composable
private fun PageWebView(
    state: WebPageState, page: TabPage, plugin: InstalledPlugin?, home: HomeViewModel, environment: TabEnvironment,
    script: String, inset: PaddingValues, visible: Boolean, openLink: (String) -> Unit, onDraft: (String) -> Unit,
) {
    val url = page.url?.takeIf { !isCrossOriginIsolationBlocked(plugin) } ?: return
    val proxy = home.proxy ?: return
    PluginWebView(
        url, proxy, page.reload,
        onLoaded = { state.loadFinished(environment) }, onFailed = state::loadFailed,
        modifier = Modifier.padding(inset), darkTheme = environment.dark,
        controller = state.controller, documentStartScript = script,
        onBridgeMessage = { origin, main, message ->
            val reply = state.tabs.core.tabHandleMessage(state.tab, main, origin.toString(), message, environment)
            reply.replyScript?.let { state.controller.evaluateJavascript(it) }
            if (reply.initialized) state.loaded = true
            when (val action = reply.action) {
                is TabBridgeAction.OpenLink -> openLink(action.url)
                is TabBridgeAction.DraftMessage -> onDraft(action.text)
                null -> Unit
            }
        },
        onOpenLink = openLink, onRendererGone = state::loadStarted, onLoadStarted = state::loadStarted,
        visible = visible, textZoom = (LocalDensity.current.fontScale * 100).roundToInt(),
    )
}

@Composable
private fun pageTakeover(tab: TabRef, tabs: TabsState, active: Boolean): PageTakeover? {
    val pluginId = (tab as? TabRef.Plugin)?.pluginId
    val tools = tabs.plugins.viewModel.installed?.plugins?.any { it.id == pluginId && it.declaresTools } == true
    val takeover = if (tools && pluginId != null) {
        PageTakeover(pluginId, tabs.plugins.viewModel.takeover, tabs::send)
    } else null
    DisposableEffect(pluginId, active) { onDispose { if (active) takeover?.end() } }
    return takeover
}

@Composable
internal fun externalLinkOpener(onFailure: (Problem) -> Unit): (String) -> Unit {
    val context = LocalContext.current
    val title = tabText("打不开浏览器", "Cannot open the browser")
    return { url ->
        try { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
        catch (failure: ActivityNotFoundException) { onFailure(Problem(title, detail = failure.message)) }
    }
}
