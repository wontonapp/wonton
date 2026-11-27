@file:Suppress("TooManyFunctions") // 栈里的页和面板页共用的网页、桥、状态都集中在这里。
package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import app.wonton.client.TabBridgeAction
import app.wonton.client.TabEnvironment
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.NativePage
import app.wonton.protocol.OpenPage
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.ProxySettings
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.music.MusicLibraryPage
import kotlin.math.roundToInt

/** 最近关掉的页面还留着 WebView 实例，重开不重载；个数是内存考虑的折中（每个 WebView 一份独立渲染进程）。 */
private const val PAGE_KEEP_ALIVE = 4

/**
 * 整页的插件网页：栈上的页面都活着，栈顶那页可见；最近关掉的几页也留在组合里（隐藏），重开同一个 key 接着用原来的 WebView。
 * 栈和关掉的页排进同一个带 key 的循环：页面在两者之间挪动时 Compose 按 key 搬动，不重建。
 */
@Composable
internal fun PluginPagesHost(
    pages: PagesState, home: HomeViewModel, appearance: AppearanceViewModel,
    active: TabRef?, onDetails: (String) -> Unit, onDraft: (String) -> Unit, bannerTop: Dp,
) {
    val stack = pages.viewModel.stack.filter { it.tab != TabRef.Chat && it.tab != TabRef.Apps }
    val closed = remember { ClosedPages() }
    val kept = remember(stack) { closed.update(stack) }
    val openTabs = stack.mapTo(HashSet()) { it.tab }
    for (open in kept + stack) {
        key(open.tab) {
            val music = open.musicLibrary()
            if (music == null) {
                PluginPageContainer(
                    open, pages, home, appearance, parked = open.tab !in openTabs,
                    active = open.tab == active, onDetails, onDraft, bannerTop,
                )
            } else if (open.tab in openTabs) {
                // 原生页没有要保活的网页：关掉就放下，重开从首页取。
                MusicLibraryPage(music, open.page.title, pages, active = open.tab == active, onDetails)
            }
        }
    }
    // 没有胶囊的整页只剩系统返回一个出口，第一次打开时教一次（[PageExitHint]）。
    if (active != null && active in openTabs && !pageKeepsCapsule(active)) PageExitHint()
}

/** 原生的音乐库只接就绪的入口页；启动中、已停止这些状态照旧由网页页的状态页说明。 */
private fun OpenPage.musicLibrary(): TabRef.Plugin? =
    (tab as? TabRef.Plugin)?.takeIf { page.native == NativePage.Music && page.state is TabState.Ready }

/** 刚关掉的页（最旧的在前）；只在栈变化时算一次，不是快照状态。 */
private class ClosedPages {
    private var last: List<OpenPage> = emptyList()
    private var closed: List<OpenPage> = emptyList()

    fun update(stack: List<OpenPage>): List<OpenPage> {
        val open = stack.mapTo(HashSet()) { it.tab }
        // widget 页不保活：关掉就是挂断，WebView 跟着销毁（ADR-0096）。
        val justClosed = last.filter { it.tab !in open && it.tab !is TabRef.Widget }
        val leaving = justClosed.mapTo(HashSet()) { it.tab }
        closed = (closed.filter { it.tab !in open && it.tab !in leaving } + justClosed).takeLast(PAGE_KEEP_ALIVE)
        last = stack
        return closed
    }
}

@Composable
private fun PluginPageContainer(
    open: OpenPage, pages: PagesState, home: HomeViewModel, appearance: AppearanceViewModel,
    parked: Boolean, active: Boolean, onDetails: (String) -> Unit, onDraft: (String) -> Unit, bannerTop: Dp,
) {
    val tab = open.tab
    val sourcePage = open.page
    val plugin = pages.installedPlugin(tab)
    val access = botWebPageAccess(tab, sourcePage.url, plugin, rememberChat(pages.core).bots)
    val page = if (access.allowed) sourcePage else sourcePage.copy(url = null, state = TabState.Unknown)
    var loginAcknowledged by remember(page.url) { mutableStateOf(false) }
    val awaitingLogin = access.disclosure && !loginAcknowledged
    val connection = sharedPageConnection(tab, pages, home)
    val state = remember { WebPageState(PlacedTab.Page(tab), pages) }
    val browserAttempted = rememberSaveable(page.url, page.reload) { mutableStateOf(false) }
    val viewport = tabViewport(active, capsule = pageKeepsCapsule(tab))
    val environment = viewport.environment
    state.park(parked)
    LaunchedEffect(parked) {
        if (parked) browserAttempted.value = false else state.resume()
    }
    val openLink = externalLinkOpener { state.problem = it }
    PageEffects(state, page, connection.connected, active, parked, appearance, environment)
    BackHandler(active) { state.back() }
    val takeover = pageTakeover(tab, pages, active)
    val disclosure = @Composable {
        BotPageDisclosure(page, active, awaitingLogin, pages, onContinue = { loginAcknowledged = true })
    }
    PageLayers(state, page, pages, connection, appearance, viewport, active, parked, takeover, bannerTop,
        onDetails, onDraft, openLink, browserAttempted, admitted = access.allowed && !awaitingLogin,
        disclosure = disclosure)
}

/** 页面顶上浮的一条短状态:连接提示由壳给,升级中由这里说。 */
@Composable
internal fun shortStatus(page: TabPage): String? =
    LocalWontonBarStatus.current.title
        ?: if (page.state is TabState.Upgrading) stringResource(R.string.tab_upgrading) else null

/** 加载中和出错时盖住网页的原生状态页：底色铺满，文字让开系统栏（[inset]）。 */
@Composable
internal fun PageStatusOverlay(
    state: WebPageState, page: TabPage, tab: TabRef, supported: Boolean, crossOriginBlocked: Boolean,
    inset: PaddingValues, onDetails: (String) -> Unit, browserAttempted: MutableState<Boolean>,
    browserAllowed: Boolean, unsupported: String? = null,
) {
    val platformBlocked = !supported || crossOriginBlocked
    if (state.covers(page) || platformBlocked || state.problem != null) {
        Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()).padding(inset)) {
            if (unsupported != null && !supported) {
                Box(Modifier.fillMaxSize().padding(WontonTheme.spacing.s32), contentAlignment = Alignment.Center) {
                    Text(unsupported, color = WontonTheme.colors.textPrimary.toColor())
                }
            } else {
                PluginTabStatus(
                    page, tab, state.pages, state.problem, supported, crossOriginBlocked, onDetails, state.place,
                    browserAttempted, browserAllowed,
                )
            }
        }
    }
}

@Composable
private fun PageEffects(
    state: WebPageState, page: TabPage, connected: Boolean, active: Boolean, parked: Boolean,
    appearance: AppearanceViewModel, environment: TabEnvironment,
) {
    val fontScale = LocalDensity.current.fontScale
    LaunchedEffect(active, connected, page.state, parked) {
        if (!parked && active && connected) {
            if (page.state is TabState.Ready) state.started = true
        }
    }
    LaunchedEffect(state.controller.customView, active) {
        if (state.controller.customView != null && !active) state.controller.closeCustomView()
    }
    LaunchedEffect(appearance, environment, fontScale, parked) {
        if (!parked && state.started) state.script(environment)?.let { state.controller.evaluateJavascript(it) }
    }
    LaunchedEffect(connected, parked) {
        if (!parked) state.controller.evaluateJavascript(state.pages.core.tabConnectionScript(state.tab))
    }
}

@Composable
internal fun PageWebView(
    state: WebPageState, page: TabPage, proxy: ProxySettings?, environment: TabEnvironment,
    script: String, visible: Boolean, openLink: (String) -> Unit, onDraft: (String) -> Unit,
    profile: String?, modifier: Modifier = Modifier,
) {
    val url = page.url?.takeIf { !page.opensInBrowser } ?: return
    proxy ?: return
    state.controller.permissionHandler = state.media::handle
    PluginWebView(
        url, proxy, page.reload + state.reloadGeneration,
        onLoaded = { state.loadFinished(environment) }, onFailed = state::loadFailed,
        modifier = modifier, darkTheme = environment.dark,
        controller = state.controller, documentStartScript = script,
        onBridgeMessage = { origin, main, message ->
            if (!state.parked) {
                val reply = state.pages.core.tabHandleMessage(
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
        onOpenLink = openLink, onRendererGone = state::loadStarted, onLoadStarted = state::loadStarted,
        visible = visible, textZoom = (LocalDensity.current.fontScale * 100).roundToInt(),
        profileName = profile, fullPage = true,
    )
}

@Composable
internal fun pageTakeover(tab: TabRef, pages: PagesState, active: Boolean): PageTakeover? {
    val pluginId = (tab as? TabRef.Plugin)?.pluginId
    val tools = pages.plugins.viewModel.installed?.plugins?.any { it.id == pluginId && it.declaresTools } == true
    val takeover = if (tools && pluginId != null) {
        PageTakeover(pluginId, pages.plugins.viewModel.takeover, pages::send)
    } else null
    DisposableEffect(pluginId, active) { onDispose { if (active) takeover?.end() } }
    return takeover
}

/** 一份网页实例的壳内状态：控制器、胶囊菜单、加载标记。[place] 区分栈里的和内容窗口里的同一页面。 */
internal class WebPageState(val place: PlacedTab, val pages: PagesState) {
    val tab: TabRef get() = when (place) {
        is PlacedTab.Page -> place.tab
        is PlacedTab.Content -> place.tab
    }
    val controller = PluginWebViewController()
    /** 网页要摄像头/麦克风时的处置；接在控制器上，页面销毁跟着失效。 */
    val media = TabMediaPermissionFlow(pages, place)
    val capsule = PageCapsuleState()
    private val lifecycle = KeepAlivePageLifecycle()
    var loaded by mutableStateOf(false)
    var failed by mutableStateOf(false)
    var started by mutableStateOf(false)
    var problem by mutableStateOf<Problem?>(null)
    var reloadGeneration by mutableStateOf(0uL)
    var documentScript: String? = null
        private set

    val parked: Boolean get() = lifecycle.parked

    fun park(value: Boolean) {
        lifecycle.setParked(value)
    }

    fun resume() {
        when (lifecycle.resume()) {
            KeepAlivePageLifecycle.ResumeAction.NONE -> Unit
            KeepAlivePageLifecycle.ResumeAction.LOAD_SUCCEEDED -> {
                pages.send(Action.Pages(PageAction.TabLoadSucceeded(place)))
            }
            KeepAlivePageLifecycle.ResumeAction.RELOAD -> {
                failed = false
                loaded = false
                problem = null
                reloadGeneration++
            }
        }
    }

    fun covers(page: TabPage): Boolean {
        val available = page.state is TabState.Ready || page.state is TabState.Upgrading
        return failed || !loaded || !available
    }

    fun script(environment: TabEnvironment, document: Boolean = false): String? = try {
        if (document) {
            pages.core.tabDocumentScript(place, environment).also { documentScript = it }
        } else {
            pages.core.tabContextScript(place, environment)
        }
    } catch (failure: CoreException) {
        // 停机、卸载中的页还没有文档脚本要，这一步抛的「输入非法」不算网页的错。
        problem = failure.problem
        null
    }

    /** 返回：全屏视频先退出，网页历史里会换页的一步照常后退，剩下的一步关掉当前页（核心弹栈）。 */
    fun back() {
        controller.backOrClose { pages.send(Action.Pages(PageAction.ClosePage)) }
    }

    fun loadStarted() {
        loaded = false; failed = false
    }

    fun loadFinished(environment: TabEnvironment, scripts: Boolean = true) {
        loaded = true
        failed = false
        if (lifecycle.loadFinished()) {
            pages.send(Action.Pages(PageAction.TabLoadSucceeded(place)))
            if (scripts) {
                script(environment)?.let { controller.evaluateJavascript(it) }
                controller.evaluateJavascript(pages.core.tabConnectionScript(tab))
            }
        }
    }

    fun loadFailed(status: Int?) {
        failed = true
        if (lifecycle.loadFailed()) pages.send(Action.Pages(PageAction.TabLoadFailed(place, status?.toUShort())))
    }
}

@Composable
internal fun externalLinkOpener(onFailure: (Problem) -> Unit): (String) -> Unit {
    val context = LocalContext.current
    val title = stringResource(R.string.tab_no_browser)
    return { url ->
        try { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
        catch (failure: ActivityNotFoundException) { onFailure(Problem(title, detail = failure.message)) }
    }
}
