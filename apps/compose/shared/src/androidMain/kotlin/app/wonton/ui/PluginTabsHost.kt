package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.webkit.WebViewFeature
import app.wonton.client.TabBridgeAction
import app.wonton.client.TabEnvironment
import app.wonton.client.TabPlatform
import app.wonton.client.TabSafeArea
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabLayout
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.WontonIcon
import java.util.TimeZone
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
                reselected = reselected == item.tab, onReselected, onDetails, onDraft,
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

    fun back(overlay: Boolean) {
        if (controller.goBack()) return
        if (overlay) tabs.send(Action.CloseTabOverlay)
        else tabs.viewModel.items.firstOrNull()?.let { tabs.send(Action.SelectTab(it.tab)) }
    }

    fun loadStarted() { loaded = false; failed = false }

    fun loadFinished(environment: TabEnvironment) {
        loaded = true
        failed = false
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
) {
    val page = item.page ?: return
    val state = remember { WebPageState(item.tab, tabs) }
    val density = LocalDensity.current
    var top by remember { mutableStateOf(0.dp) }
    val edge = page.layout == TabLayout.EDGE_TO_EDGE
    var retainedBottom by remember { mutableStateOf(bottom) }
    SideEffect { if (active) retainedBottom = bottom }
    val environment = tabEnvironment(top, retainedBottom, edge)
    val script = remember(page.url, appearance, environment) { state.script(environment, document = true) }
    val openLink = externalLinkOpener { state.problem = it }
    val supported = remember { webViewSupportsTabs() }
    PageEffects(state, page, home, active, appearance, environment)
    ReselectedEffect(state, reselected, onReselected)
    val back = { state.back(overlay) }
    TabBackHandler(state, active, overlay, back)
    val cover = state.covers(page)
    val inset = if (edge) PaddingValues(0.dp) else PaddingValues(top = top, bottom = retainedBottom)
    val takeover = pageTakeover(item.tab, tabs, active)
    Box(Modifier.fillMaxSize().imePadding()) {
        if (state.started && supported && script != null) {
            PageWebView(state, page, home, environment, script, inset, active && !cover, openLink, onDraft)
        }
        if (active) {
            if (cover || !supported || state.problem != null) {
                Box(Modifier.padding(top = top, bottom = retainedBottom).fillMaxSize()
                    .background(WontonTheme.colors.canvas.toColor())) {
                    PluginTabStatus(page, item.tab, tabs, state.problem, supported, onDetails)
                }
            }
            Column(Modifier.align(Alignment.TopStart).onSizeChanged { top = with(density) { it.height.toDp() } }) {
                PluginTabBar(page, state.controller.canGoBack, overlay, back,
                    { tabs.send(Action.CloseTabOverlay) }, takeover) {
                    state.problem = null
                    tabs.send(Action.ReloadTab(item.tab))
                }
                takeover?.let { TakeoverNotice(it) }
            }
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
private fun TabBackHandler(state: WebPageState, active: Boolean, overlay: Boolean, back: () -> Unit) {
    val canLeave = overlay || state.tabs.viewModel.items.firstOrNull()?.tab != state.tab
    BackHandler(active && (state.controller.canGoBack || canLeave), back)
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
    LaunchedEffect(appearance, environment, page.fullscreen, page.display, fontScale) {
        if (state.started) state.script(environment)?.let { state.controller.evaluateJavascript(it) }
    }
    LaunchedEffect(home.status) { state.controller.evaluateJavascript(state.tabs.core.tabConnectionScript()) }
}

@Composable
private fun PageWebView(
    state: WebPageState, page: TabPage, home: HomeViewModel, environment: TabEnvironment,
    script: String, inset: PaddingValues, visible: Boolean, openLink: (String) -> Unit, onDraft: (String) -> Unit,
) {
    val url = page.url ?: return
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
private fun tabEnvironment(top: Dp, bottom: Dp, edge: Boolean): TabEnvironment {
    val density = LocalDensity.current
    val safe = WindowInsets.safeDrawing
    val configuration = LocalConfiguration.current
    return TabEnvironment(
        configuration.locales[0].toLanguageTag(), TimeZone.getDefault().id, TabPlatform.ANDROID,
        WontonTheme.isDark, top.value.toDouble(), bottom.value.toDouble(),
        TabSafeArea(
            if (edge) (safe.getTop(density) / density.density).toDouble() else 0.0,
            0.0, if (edge) (safe.getBottom(density) / density.density).toDouble() else 0.0, 0.0,
        ),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PluginTabBar(
    page: TabPage, canGoBack: Boolean, overlay: Boolean, onBack: () -> Unit,
    onClose: () -> Unit, takeover: PageTakeover?, onReload: () -> Unit,
) {
    val colors = WontonTheme.colors
    val status = LocalWontonBarStatus.current
    Column(Modifier.fillMaxWidth().background(colors.surface.toColor())) {
        TopAppBar(
            title = { Text(status.title ?: page.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = { if (canGoBack) WontonIconButton(WontonIcon.Back, tabText("返回", "Back"), onBack) },
            actions = {
                takeover?.let { TakeoverButton(it) }
                WontonIconButton(WontonIcon.Refresh, tabText("重新加载", "Reload"), onReload)
                if (overlay) WontonIconButton(WontonIcon.Close, tabText("关闭", "Close"), onClose)
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = colors.surface.toColor()),
        )
        status.line?.let { Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) { it() } }
        if (page.state is TabState.Upgrading) Text(
            tabText("正在升级…", "Updating…"), Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            color = colors.textSecondary.toColor(), style = WontonTheme.typography.supporting,
        )
    }
}

private fun webViewSupportsTabs(): Boolean = listOf(
    WebViewFeature.WEB_MESSAGE_LISTENER, WebViewFeature.DOCUMENT_START_SCRIPT,
    WebViewFeature.PROXY_OVERRIDE, WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS,
).all(WebViewFeature::isFeatureSupported)

@Composable
internal fun externalLinkOpener(onFailure: (Problem) -> Unit): (String) -> Unit {
    val context = LocalContext.current
    val title = tabText("打不开浏览器", "Cannot open the browser")
    return { url ->
        try { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
        catch (failure: ActivityNotFoundException) { onFailure(Problem(title, detail = failure.message)) }
    }
}
