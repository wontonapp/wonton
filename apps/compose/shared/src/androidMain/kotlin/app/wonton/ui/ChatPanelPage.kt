package app.wonton.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.webkit.WebViewFeature
import app.wonton.client.TabBridgeAction
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PanelItem
import app.wonton.protocol.PanelScope
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.TabLayout
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import kotlin.math.roundToInt

/**
 * 面板里的一页：网页一直按整页排版（第 1 级只是被聊天页盖住一截，不改大小），
 * 只在第 2 级挂胶囊。公网网页没有桥、没有注入脚本，单独一个网站数据存储（ADR-0052）。
 */
@Suppress("CyclomaticComplexMethod") // 一页的网页、状态、胶囊和手势都在这，摊平反而要来回跳。
@Composable
internal fun PanelPage(
    item: PanelItem,
    scope: PanelScope,
    panel: ChatPanelState,
    tabs: TabsState,
    home: HomeViewModel,
    appearance: AppearanceViewModel,
    active: Boolean,
    current: Boolean,
    touch: PanelPageTouch,
    modifier: Modifier,
    onDraft: (String) -> Unit,
    onDetails: (String) -> Unit,
) {
    val page = remember(item.page) { item.page.copy(fullscreen = true) }
    val tab = item.tab
    val public = tab is TabRef.Url
    val place = remember(scope, tab) { PlacedTab.Panel(scope, tab) }
    val state = remember(place) { WebPageState(place, tabs) }
    val safe = tabSafeInsets()
    val direction = LocalLayoutDirection.current
    val density = LocalDensity.current
    val edge = page.layout == TabLayout.EDGE_TO_EDGE
    val cover = if (panel.level == PanelLevel.FULL) panel.safeBottom else panel.strip
    val environment = tabEnvironment(
        safe.calculateTopPadding(), with(density) { cover.toDp() }, edge, safe,
    )
    val inset = if (edge) {
        PaddingValues(0.dp)
    } else {
        PaddingValues(top = safe.calculateTopPadding(), bottom = safe.calculateBottomPadding())
    }
    val environmentKey = environment
    val canConnect = public || home.status is HomeStatus.Connected
    LaunchedEffect(active, canConnect, page.state) {
        if (active && canConnect && page.state is TabState.Ready) state.started = true
    }
    val script = remember(page.url, page.fullscreen, page.display, appearance, environmentKey, public) {
        if (public) null else state.script(environmentKey, document = true)
    }
    LaunchedEffect(appearance, environmentKey, page.fullscreen, page.display, public) {
        if (!public && state.started) state.script(environmentKey)?.let { state.controller.evaluateJavascript(it) }
    }
    LaunchedEffect(home.status) {
        if (!public) state.controller.evaluateJavascript(state.tabs.core.tabConnectionScript(state.tab))
    }
    val capsule = state.capsule
    TabCapsuleEffects(capsule, current && panel.level == PanelLevel.FULL, true)
    val capsuleObserver = rememberCapsuleTouchObserver(capsule, current && panel.level == PanelLevel.FULL)
    DisposableEffect(state.controller, capsuleObserver) {
        state.controller.touchObserver = { event ->
            if (panel.level == PanelLevel.FULL) capsuleObserver?.invoke(event) else touch.observe(event)
        }
        if (current) panel.controller = state.controller
        onDispose {
            state.controller.touchObserver = null
            if (panel.controller === state.controller) panel.controller = null
        }
    }
    val takeover = pageTakeover(tab, tabs, current && panel.level == PanelLevel.FULL)
    val supported = remember(public) {
        if (public) {
            publicProfilesSupported() && WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)
        } else {
            tabFeaturesSupported()
        }
    }
    val crossOriginBlocked = !public && page.opensInBrowser
    val openLink = externalLinkOpener { state.problem = it }
    androidx.compose.foundation.layout.Box(modifier.fillMaxSize()) {
        val loadable = page.url?.takeIf { !crossOriginBlocked } != null && (public || home.proxy != null)
        if (state.started && supported && loadable) {
            PageWebView(
                state, page, home, environmentKey, script.orEmpty(), inset,
                visible = active, openLink = openLink, onDraft = onDraft, publicPage = public,
            )
        }
        PageStatusOverlay(
            state, page, tab, supported, crossOriginBlocked, inset, onDetails,
            unsupported = if (public) stringResource(R.string.panel_public_unsupported) else null,
        )
        if (panel.level == PanelLevel.FULL && current) {
            PluginPageChrome(tab, page, tabs, safe, 0.dp, shortStatus(page), capsule, state.controller, takeover,
                onClose = { panel.move(PanelLevel.CLOSED) },
                onReload = { tabs.send(Action.ReloadTab(place)) },
                onDetails = onDetails,
                onEditPanel = { panel.editing = true })
        }
    }
    DisposableEffect(current) { onDispose { if (current) takeover?.end() } }
}

/**
 * 面板页的网页：公网网页不注入脚本、不挂桥，单独的数据存储；插件页和底栏一样走隧道和桥。
 * 给 `PageWebView` 用，`publicPage` 由这里传。
 */
@Composable
private fun PageWebView(
    state: WebPageState,
    page: app.wonton.protocol.TabPage,
    home: HomeViewModel,
    environment: app.wonton.client.TabEnvironment,
    script: String,
    inset: PaddingValues,
    visible: Boolean,
    openLink: (String) -> Unit,
    onDraft: (String) -> Unit,
    publicPage: Boolean,
) {
    val url = page.url ?: return
    val proxy = if (publicPage) null else home.proxy ?: return
    val density = LocalDensity.current
    if (!publicPage) state.controller.permissionHandler = state.media::handle
    PluginWebView(
        url, proxy, page.reload,
        onLoaded = { state.loadFinished(environment) },
        onFailed = state::loadFailed,
        modifier = Modifier.padding(inset),
        darkTheme = environment.dark,
        controller = state.controller,
        documentStartScript = script,
        onBridgeMessage = { origin, main, message ->
            if (!publicPage) {
                val reply = state.tabs.core.tabHandleMessage(
                    state.place, main, origin.toString(), message, environment,
                )
                reply.replyScript?.let { state.controller.evaluateJavascript(it) }
                if (reply.initialized) state.loaded = true
                when (val action = reply.action) {
                    is TabBridgeAction.OpenLink -> openLink(action.url)
                    is TabBridgeAction.DraftMessage -> onDraft(action.text)
                    null -> Unit
                }
            }
        },
        onOpenLink = openLink,
        onRendererGone = state::loadStarted,
        onLoadStarted = state::loadStarted,
        visible = visible,
        textZoom = (density.fontScale * 100).roundToInt(),
        publicPage = publicPage,
    )
}
