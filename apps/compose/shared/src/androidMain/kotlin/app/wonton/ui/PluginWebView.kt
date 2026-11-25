package app.wonton.ui

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.util.Log
import android.view.View
import android.view.MotionEvent
import android.view.ViewGroup
import android.view.ContextThemeWrapper
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.HttpAuthHandler
import android.webkit.PermissionRequest
import android.webkit.WebResourceError
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.result.ActivityResultLauncher
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.ProfileStore
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.wonton.protocol.ProxySettings
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.media.PageMediaPage
import java.util.concurrent.Executor

private const val TAG = "WontonWeb"

/** 和 `wonton-tunnel` 的 `ERROR_HEADER` 一致。 */
private const val TUNNEL_ERROR_HEADER = "x-wonton-tunnel-error"

// 代理给壳的用户可读错误类别，取值见核心 `TunnelError::category`。
private const val CATEGORY_ERROR_HEADER = "x-wonton-error"

// 只有这些取值是本地代理回的；网关的响应也带 x-wonton-error，取值不同。
private val PROXY_ERROR_CATEGORIES = setOf(
    "tunnel-unavailable", "upstream-unavailable", "unauthorized", "protocol",
)

/** 只有服务端错误才算加载失败;4xx 是插件自己的内容。 */
private const val HTTP_SERVER_ERROR = 500

// 和 iOS 一样，页面写了 user-scalable=no 或较小的 maximum-scale 也能双指缩放；Android WebView 聚焦输入框不放大，
// 去掉上限不会带来聚焦放大。WebView 124 实测：onPageFinished 时再改 viewport 已不生效，要在 DOMContentLoaded 改。
private val pinchZoomScript = """
    document.addEventListener('DOMContentLoaded', () => {
      const meta = document.querySelector('meta[name="viewport"]');
      if (!meta) return;
      const directives = meta.content.split(/[,;]/).map(value => value.trim());
      const scalable = directives.filter(directive => {
        const [key, value] = directive.split('=').map(part => part.trim().toLowerCase());
        return !(key === 'user-scalable' && value === 'no') &&
          !(key === 'maximum-scale' && Number.parseFloat(value) < 5);
      });
      if (scalable.length !== directives.length) meta.content = scalable.join(',');
    }, { once: true });
""".trimIndent()

/** UI 线程能力引用；脚本仅接收核心返回值，不在 Android 壳中解释桥协议。 */
class PluginWebViewController {
    internal var view: WebView? = null
    internal var touchObserver: ((MotionEvent) -> Unit)? = null
    /** 网页要摄像头/麦克风时的处置（[TabMediaPermissionFlow]）；没接的页面一律拒绝。 */
    internal var permissionHandler: ((PermissionRequest) -> Unit)? = null
    internal var fileChooserLauncher: ActivityResultLauncher<Intent>? = null
    internal var pendingFileChooser: ValueCallback<Array<Uri>>? = null
    var canGoBack by mutableStateOf(false)
        private set
    var customView: View? by mutableStateOf(null)
        private set
    /** 页面自己的顶栏颜色（[PageThemeColor]）；没有可用的颜色时是 null。 */
    var pageColor: Color? by mutableStateOf(null)
        internal set
    /** 本页的历史足迹；[back] 用它认出不会换页的一步。 */
    internal val history = PageHistory()
    private var customViewCallback: WebChromeClient.CustomViewCallback? = null

    internal fun onPermissionRequest(request: PermissionRequest) {
        val handler = permissionHandler
        if (handler == null) request.deny() else handler(request)
    }

    /** 退出网页视频全屏；胶囊和系统返回都走原生回调，不接管 Activity 的窗口。 */
    fun closeCustomView(): Boolean {
        val current = customView ?: return false
        val callback = customViewCallback
        customView = null
        customViewCallback = null
        (current.parent as? ViewGroup)?.removeView(current)
        callback?.onCustomViewHidden()
        return true
    }

    internal fun showCustomView(owner: WebView, custom: View, callback: WebChromeClient.CustomViewCallback) {
        if (view !== owner || customView != null) {
            callback.onCustomViewHidden()
            return
        }
        customViewCallback = callback
        customView = custom
    }


    internal fun release(owner: WebView) {
        if (view !== owner) return
        view = null
        history.clear()
        canGoBack = false
        closeCustomView()
        pendingFileChooser?.onReceiveValue(null)
        pendingFileChooser = null
        fileChooserLauncher = null
    }

    fun goBack(): Boolean {
        val current = view
        val available = current?.canGoBack() == true
        if (available) current.goBack()
        return available
    }

    fun scrollToTop() { view?.scrollTo(0, 0) }

    /** 页面尚未创建时返回 false，父层可等 onLoaded 后重送 context/connection。 */
    fun evaluateJavascript(script: String): Boolean {
        val current = view ?: return false
        current.evaluateJavascript("if (window.wontonBridge) {\n$script\n}", null)
        return true
    }

    internal fun updateHistory() {
        canGoBack = view?.canGoBack() == true
    }


    /**
     * 系统返回和胶囊菜单的“返回”：视频全屏先退出，网页历史里会换页的一步照常后退；同一文档里的记录
     * （重定向、同地址）直接关页，由调用方经 [backOrClose] 发 ClosePage。
     */
    internal fun back(): PluginBackTarget {
        val step = history.step(view)
        val target = pluginBackTarget(customView != null, step)
        // 复现用例靠这几项判断返回键走到哪一步；两个地址都经过 logUrl，日志里不会有授权码。
        val where = "sameDocument=${step?.sameDocument} previous=${step?.previous} page=${history.pagePath()}"
        Log.i(TAG, "back: target=$target $where")
        when (target) {
            PluginBackTarget.CloseCustomView -> closeCustomView()
            PluginBackTarget.History -> view?.goBack()
            PluginBackTarget.ClosePage -> Unit
        }
        return target
    }
}

/**
 * 本页的历史足迹：当前地址，以及哪些地址是重定向来的（来源 → 落地，去掉片段）。
 * 判断“上一历史步会不会换页”用地址比较，不靠等：同一地址、或一条会跳回本页的重定向都算同一页。
 * 重定向这一路是保险：Chromium 把服务端跳转的落地地址顶替进那一条历史，来源不在历史里，
 * 模拟器上还没见它生效过（见 PR #903 的说明）。
 */
internal class PageHistory {
    private var currentUrl: String? = null
    /** 正在加载的文档地址；加载完和最终地址不同就是一条重定向。 */
    private var startedUrl: String? = null
    private val redirects = mutableMapOf<String, String>()

    fun started(url: String) {
        startedUrl = url
        currentUrl = url
    }

    fun finished(url: String) {
        val started = startedUrl ?: return
        startedUrl = null
        val from = started.substringBefore('#')
        val to = url.substringBefore('#')
        if (from != to) redirects[from] = to
    }

    fun visited(url: String?) {
        if (url != null) currentUrl = url
    }

    /** 当前页前面最近的一步；没有历史、或拿不到它的地址时 null。 */
    fun step(view: WebView?): HistoryStep? {
        val list = view?.takeIf { it.canGoBack() }?.copyBackForwardList()
        val previous = list?.getItemAtIndex(list.currentIndex - 1)?.url
        return previous?.let { HistoryStep(sameDocument = isSameDocument(it), previous = logUrl(it)) }
    }

    internal fun isSameDocument(previous: String): Boolean {
        val current = currentUrl ?: return false
        return previous == current || redirects[previous.substringBefore('#')] == current.substringBefore('#')
    }

    /** 日志用：只到路径，去掉查询串（可能有授权码）和片段。 */
    fun pagePath(): String? = currentUrl?.let(::logUrl)

    fun clear() {
        currentUrl = null
        startedUrl = null
        redirects.clear()
    }
}

@Composable
fun rememberPluginWebViewController(): PluginWebViewController = remember { PluginWebViewController() }

/** 精确 origin（包含非默认端口），同时用于桥监听和主框架导航策略。 */
fun pluginOrigin(url: String): String {
    val uri = Uri.parse(url)
    val scheme = uri.scheme?.lowercase()
    require(scheme == "http" || scheme == "https") { "plugin URL must be HTTP(S)" }
    val host = requireNotNull(uri.host).lowercase()
    val port = uri.port.takeUnless { it == -1 || (scheme == "http" && it == 80) || (scheme == "https" && it == 443) }
    return "$scheme://$host${port?.let { ":$it" }.orEmpty()}"
}

private class WebViewSnapshot {
    var url: String? = null
    var reload: ULong? = null
    var recoveryUrl: String? = null
}

/**
 * 固定标签页和 overlay 共用的无导航栏容器。父层保留 composition 即保留历史和滚动位置。
 * reload/url 改变才导航；其他重组只更新回调。桥来源来自 WebView，不读取消息正文。
 * document-start 更新仅影响未来文档；当前页 context 由父层通过 controller 注入。
 */
/** 公网网页单独一个网站数据存储，不和插件页共用 Cookie 和存储（ADR-0052）。 */
private const val PUBLIC_PROFILE = "wonton-public"

/** 设备支持多 Profile 才打公网网页，不支持时不退回共用插件页的存储。 */
internal fun publicProfilesSupported(): Boolean = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)

private val publicProfile by lazy(LazyThreadSafetyMode.NONE) {
    ProfileStore.getInstance().getOrCreateProfile(PUBLIC_PROFILE)
}

private data class WebLayer(val profile: String?, val multiProfile: Boolean)

/** 本页该用的 Profile；插件页不在本机代理里、公网页不支持多 Profile 时返回 null。 */
@Composable
private fun rememberWebLayer(proxy: ProxySettings?, profileName: String?, publicPage: Boolean): WebLayer? {
    val route = LocalPluginProxy.current
    val account = LocalPluginAccount.current
    // 公网页不取插件账号的 Profile：它挂独立的公网 Profile，由 factory 里 setProfile（ADR-0052）。
    val profile = if (publicPage) null else profileName ?: account?.let { pluginProfileName(it) }
    val multiProfile = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)
    // 公网页不走本机代理（proxy 为 null），挂在独立的公网 Profile 上；插件页必须在本机代理路由里。
    val usable = if (publicPage) multiProfile else proxy != null && route?.contains(proxy) == true
    val unsupported = profileName != null && !multiProfile
    return if (!usable || unsupported) {
        null
    } else {
        WebLayer(profile, multiProfile)
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
fun PluginWebView(
    url: String,
    proxy: ProxySettings?,
    reload: ULong,
    onLoaded: () -> Unit,
    onFailed: (Int?) -> Unit,
    modifier: Modifier = Modifier,
    darkTheme: Boolean = WontonTheme.isDark,
    controller: PluginWebViewController = rememberPluginWebViewController(),
    documentStartScript: String = "",
    allowedOrigins: Set<String> = setOf(pluginOrigin(url)),
    onBridgeMessage: (Uri, Boolean, String) -> Unit = { _, _, _ -> },
    onOpenLink: (String) -> Unit,
    onRendererGone: () -> Unit = { onFailed(null) },
    onLoadStarted: () -> Unit = {},
    visible: Boolean = true,
    textZoom: Int = 100,
    publicPage: Boolean = false,
    profileName: String? = null,
    fullPage: Boolean = false,
) {
    val loaded by rememberUpdatedState(onLoaded)
    val failed by rememberUpdatedState(onFailed)
    val bridge by rememberUpdatedState(onBridgeMessage)
    val openLink by rememberUpdatedState(onOpenLink)
    val gone by rememberUpdatedState(onRendererGone)
    val loadStarted by rememberUpdatedState(onLoadStarted)
    rememberFileChooserLauncher(controller)
    val canvas = WontonTheme.colors.canvas.toColor().toArgb()
    var generation by remember { mutableIntStateOf(0) }
    val snapshot = remember(controller) { WebViewSnapshot() }
    var proxyReady by remember { mutableStateOf<ProxySettings?>(null) }
    DisposableEffect(proxy) {
        if (proxy == null) {
            proxyReady = null
            return@DisposableEffect onDispose { }
        }
        var active = true
        overrideProxy(proxy) { if (active) proxyReady = proxy }
        onDispose { active = false }
    }
    val layer = rememberWebLayer(proxy, profileName, publicPage) ?: return
    key(controller, generation, layer.profile) {
        AndroidView(
            modifier = modifier.fillMaxSize(),
            factory = { context ->
                val wrapped = pluginContext(context, darkTheme)
                val debuggable = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
                WebView.setWebContentsDebuggingEnabled(debuggable)
                ThemedPluginWebView(wrapped, darkTheme, controller).apply {
                    layoutParams = ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT,
                    )
                    settings.javaScriptEnabled = true
                    settings.domStorageEnabled = true
                    // 插件页换曲时在新元素上自动播放，等不到用户手势；页面本来就是主人打开的。
                    settings.mediaPlaybackRequiresUserGesture = false
                    settings.builtInZoomControls = true
                    settings.displayZoomControls = false
                    if (publicPage) {
                        // 公网页优先：账号非空时 layer.profile 也不该指到插件 Profile（ADR-0052）。
                        WebViewCompat.setProfile(this, publicProfile.getName())
                    } else if (layer.multiProfile && layer.profile != null) {
                        WebViewCompat.setProfile(this, layer.profile)
                    }
                    configureAppearance(this, canvas, darkTheme)
                    // 模拟器用 SwiftShader 画 WebView，整屏网页常超显存配额，图一片都不出（渲染进程
                    // 仍可能崩）；调试包在模拟器上改软件层，真机照旧走硬件。
                    setSoftwareLayerIfEmulator(context)
                    controller.view = this
                    webChromeClient = PluginChromeClient(this, controller)
                    webViewClient = ProxyAuthClient(proxy, reload, url, controller, snapshot,
                        { loaded() }, { failed(it) }, { loadStarted() }, { openLink(it) }, {
                            generation++
                            gone()
                        })
                }
            },
            onRelease = { view ->
                view.disposePage()
                if (!view.rendererGone) view.destroy()
            },
            update = { view ->
                view.fullPage = fullPage
                if (!publicPage) {
                    view.updateBridge(allowedOrigins, documentStartScript) { origin, main, message ->
                        bridge(origin, main, message)
                    }
                }
                configureBackground(view, canvas)
                view.updateTheme(darkTheme, canvas)
                view.wantedVisible = visible
                view.visibility = if (visible) android.view.View.VISIBLE else android.view.View.INVISIBLE
                if (view.settings.textZoom != textZoom) view.settings.textZoom = textZoom
                val client = view.webViewClient as ProxyAuthClient
                client.proxy = proxy
                if (proxy == null || proxyReady == proxy) client.navigate(view, url, reload)
            },
        )
    }
}

private fun pluginContext(context: android.content.Context, dark: Boolean): ContextThemeWrapper {
    val wrapped = ContextThemeWrapper(context, pluginTheme(dark))
    val config = Configuration(context.resources.configuration)
    config.uiMode = (config.uiMode and Configuration.UI_MODE_NIGHT_MASK.inv()) or
        if (dark) Configuration.UI_MODE_NIGHT_YES else Configuration.UI_MODE_NIGHT_NO
    wrapped.applyOverrideConfiguration(config)
    return wrapped
}

private fun pluginTheme(dark: Boolean): Int = if (dark) android.R.style.Theme_Material_NoActionBar
    else android.R.style.Theme_Material_Light_NoActionBar

private fun WebView.setSoftwareLayerIfEmulator(context: Context) {
    val debug = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
    if (debug && Build.HARDWARE == "ranchu") setLayerType(View.LAYER_TYPE_SOFTWARE, null)
}

private class ThemedPluginWebView(
    private val wrapped: ContextThemeWrapper,
    private var dark: Boolean,
    private val controller: PluginWebViewController,
) : WebView(wrapped) {
    var rendererGone = false
    var wantedVisible = false
    private var disposed = false
    private var bridgeOrigins: Set<String>? = null
    private var startScript: String? = null
    private var scriptHandler: androidx.webkit.ScriptHandler? = null
    private val pageMedia = PageMediaPage(this)
    private val pageColor = PageThemeColor(this) { color -> if (controller.view === this) controller.pageColor = color }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        controller.touchObserver?.invoke(event)
        return super.dispatchTouchEvent(event)
    }

    /**
     * 整页（上面让出状态栏那一条的页）。它不要外层的回弹：内容再短也能把整页连同它固定的顶栏拉出去一截，页面里自己的
     * 滚动容器照旧。取色脚本也只给它装：面板和引导页没有那一条，装了只是白算。
     */
    var fullPage = false
        set(value) {
            field = value
            if (value) overScrollMode = View.OVER_SCROLL_NEVER
        }

    fun disposePage() {
        if (disposed) return
        disposed = true
        scriptHandler?.remove()
        scriptHandler = null
        pageMedia.dispose()
        pageColor.dispose()
        if (bridgeOrigins != null) WebViewCompat.removeWebMessageListener(this, "wontonBridge")
        bridgeOrigins = null
        controller.release(this)
        webChromeClient = null
    }

    fun updateBridge(origins: Set<String>, script: String, onMessage: (Uri, Boolean, String) -> Unit) {
        require(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER))
        require(WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT))
        pageMedia.update(origins)
        if (fullPage) pageColor.update(origins)
        val originsChanged = bridgeOrigins != origins
        if (originsChanged) {
            if (bridgeOrigins != null) WebViewCompat.removeWebMessageListener(this, "wontonBridge")
            WebViewCompat.addWebMessageListener(this, "wontonBridge", origins) { _, message, origin, main, _ ->
                if (message.type == androidx.webkit.WebMessageCompat.TYPE_STRING) {
                    message.data?.let { onMessage(origin, main, it) }
                }
            }
        }
        if (originsChanged || startScript != script) {
            scriptHandler?.remove()
            scriptHandler = WebViewCompat.addDocumentStartJavaScript(
                this, "if (window === window.top) {\n$script\n$pinchZoomScript\n}", origins,
            )
            startScript = script
        }
        if (originsChanged) bridgeOrigins = origins.toSet()
    }

    @Suppress("DEPRECATION")
    fun updateTheme(nextDark: Boolean, canvas: Int) {
        if (dark == nextDark) return
        dark = nextDark
        wrapped.setTheme(pluginTheme(dark))
        val config = Configuration(wrapped.resources.configuration)
        config.uiMode = (config.uiMode and Configuration.UI_MODE_NIGHT_MASK.inv()) or
            if (dark) Configuration.UI_MODE_NIGHT_YES else Configuration.UI_MODE_NIGHT_NO
        wrapped.resources.updateConfiguration(config, wrapped.resources.displayMetrics)
        onConfigurationChanged(config)
        configureAppearance(this, canvas, dark)
    }
}


private fun configureBackground(view: WebView, canvas: Int) {
    view.setBackgroundColor(canvas)
}

@Suppress("DEPRECATION")
private fun configureAppearance(view: WebView, canvas: Int, dark: Boolean) {
    configureBackground(view, canvas)
    val force = WebViewFeature.isFeatureSupported(WebViewFeature.FORCE_DARK)
    val strategy = WebViewFeature.isFeatureSupported(WebViewFeature.FORCE_DARK_STRATEGY)
    val algorithmic = WebViewFeature.isFeatureSupported(WebViewFeature.ALGORITHMIC_DARKENING)
    if (strategy) {
        WebSettingsCompat.setForceDarkStrategy(view.settings, WebSettingsCompat.DARK_STRATEGY_WEB_THEME_DARKENING_ONLY)
    }
    if (force) {
        val mode = if (dark) WebSettingsCompat.FORCE_DARK_ON else WebSettingsCompat.FORCE_DARK_OFF
        WebSettingsCompat.setForceDark(view.settings, mode)
    }
    // targetSdk >= 33 忽略旧 FORCE_DARK；禁止算法反色，尊重网页主题，不把缺少 meta 的页面强行反色。
    if (algorithmic) WebSettingsCompat.setAlgorithmicDarkeningAllowed(view.settings, false)
    Log.i(TAG, "WebView theme dark=$dark webThemeOnly=$strategy forceDark=$force " +
        "algorithmicSupported=$algorithmic algorithmicAllowed=false")
}

private fun overrideProxy(proxy: ProxySettings, onReady: () -> Unit) {
    check(
        WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS),
    ) {
        "this device's WebView is too old to set a proxy per domain, app pages can't open"
    }
    val config = ProxyConfig.Builder()
        .addProxyRule("${proxy.host}:${proxy.port}")
        .addBypassRule(proxy.domain)
        .addBypassRule("*.${proxy.domain}")
        .setReverseBypassEnabled(true)
        .build()
    val sameThread = Executor { it.run() }
    ProxyController.getInstance().setProxyOverride(config, sameThread) {
        Log.i(TAG, "WebView proxy set to ${proxy.host}:${proxy.port}")
        onReady()
    }
}

/** 只把本地代理的凭据交给本地代理自己：别的主机（例如插件网页）要认证时不给。 */
private class ProxyAuthClient(
    var proxy: ProxySettings?,
    private var reload: ULong,
    private var url: String,
    private val controller: PluginWebViewController,
    private val snapshot: WebViewSnapshot,
    private val onLoaded: () -> Unit,
    private val onFailed: (Int?) -> Unit,
    private val onLoadStarted: () -> Unit,
    private val onOpenLink: (String) -> Unit,
    private val onRendererGone: () -> Unit,
) : WebViewClient() {
    private var failedThisLoad = false

    /** HTTP 错误的 URL。错误回调早于 onPageStarted 到达（实测），同 URL 的 pageStarted 不能清失败标记。 */
    private var failedUrl: String? = null
    private var started = false
    private var navigatedProxy: ProxySettings? = null
    private var currentUrl = url

    fun navigate(view: WebView, nextUrl: String, nextReload: ULong) {
        val sameNavigation = url == nextUrl && reload == nextReload
        if (started && sameNavigation && navigatedProxy == proxy) return
        val restore = !started && snapshot.url == nextUrl && snapshot.reload == nextReload
        navigatedProxy = proxy
        started = true
        url = nextUrl
        reload = nextReload
        snapshot.url = nextUrl
        snapshot.reload = nextReload
        // 重试（reload 变化）是新一次加载：这里清掉上一轮失败，成功由 onPageFinished 恢复。
        failedThisLoad = false
        failedUrl = null
        val recovery = snapshot.recoveryUrl.takeIf { restore }
        snapshot.recoveryUrl = null
        view.loadUrl(recovery ?: nextUrl)
    }

    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
        if (!request.isForMainFrame) return false
        val uri = request.url
        val isHttp = uri.scheme == "http" || uri.scheme == "https"
        // OIDC 登录会经过核心的这段路径；系统浏览器没有隧道和设备身份，走不完。
        val coreLogin = uri.host.equals(proxy?.domain, ignoreCase = true) &&
            uri.path.orEmpty().startsWith("/_wonton/oidc/")
        val authHost = proxy?.let { "auth.${it.domain}" }
        val staysInside = isHttp && (pluginOrigin(uri.toString()) == pluginOrigin(url) ||
            uri.host.equals(authHost, ignoreCase = true) || coreLogin)
        if (isHttp && !staysInside) onOpenLink(uri.toString())
        return !staysInside
    }

    override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
        if (url != null) currentUrl = url
        controller.history.visited(url)
        controller.updateHistory()
    }

    override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
        Log.w(TAG, "WebView renderer exited, didCrash=${detail.didCrash()}")
        snapshot.recoveryUrl = currentUrl
        (view as ThemedPluginWebView).rendererGone = true
        view.disposePage()
        (view.parent as? ViewGroup)?.removeView(view)
        view.destroy()
        onRendererGone()
        return true
    }

    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
        controller.closeCustomView()
        controller.history.started(url)
        if (url == failedUrl) {
            // HTTP 错误回调已到达并标记本轮失败：保持失败和隐藏，不当作新加载开始。
            currentUrl = url
            return
        }
        view.visibility = android.view.View.INVISIBLE
        currentUrl = url
        failedThisLoad = false
        onLoadStarted()
    }


    override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
        if (request.isForMainFrame) {
            // 错误页上看不出是哪一层拒的（明文策略、代理、隧道），原因只在这里；地址照 onPageFinished 只记到路径。
            val page = request.url.buildUpon().clearQuery().fragment(null).build()
            Log.w(TAG, "page failed to load, page=$page code=${error.errorCode} reason=${error.description}")
            failedUrl = request.url.toString()
            failed(view)
        }
    }

    /** 网关失败与 5xx 才算加载失败；4xx 是插件自己的内容（登录页、站内 404），照常显示。
     * 规则与 iOS 的 `tunnelFailed || statusCode >= 500` 一致（TabWebNavigation.swift）。 */
    override fun onReceivedHttpError(view: WebView, request: WebResourceRequest, response: WebResourceResponse) {
        if (!request.isForMainFrame) return
        val headers = response.responseHeaders.orEmpty()
        // 类别由代理给（x-wonton-error）。服务端的网关也用这个头名，只认代理自己的取值：
        // tunnel-unavailable 是没拿到响应，upstream-unavailable 拿到了响应（服务器在，上游没就绪），
        // 照实交状态码给核心判断。
        val category = headers.entries
            .firstOrNull { it.key.equals(CATEGORY_ERROR_HEADER, ignoreCase = true) }?.value?.lowercase()
        val proxyCategory = category in PROXY_ERROR_CATEGORIES
        val fromProxy = proxyCategory || headers.keys.any { it.equals(TUNNEL_ERROR_HEADER, ignoreCase = true) }
        if (!fromProxy && response.statusCode < HTTP_SERVER_ERROR) return
        failedUrl = request.url.toString()
        val status = when (category) {
            "upstream-unavailable" -> response.statusCode
            "tunnel-unavailable", "unauthorized", "protocol" -> null
            else -> if (fromProxy) null else response.statusCode
        }
        failed(view, status)
    }

    private fun failed(view: WebView, status: Int? = null) {
        view.visibility = android.view.View.INVISIBLE
        controller.closeCustomView()
        failedThisLoad = true
        onFailed(status)
    }

    override fun onReceivedHttpAuthRequest(view: WebView, handler: HttpAuthHandler, host: String, realm: String) {
        val current = proxy
        if (current != null && (host == current.host || host == "${current.host}:${current.port}")) {
            handler.proceed(current.username, current.password)
        } else {
            super.onReceivedHttpAuthRequest(view, handler, host, realm)
        }
    }

    override fun onPageFinished(view: WebView, url: String) {
        failedUrl = null
        // 只记到路径：查询串和片段里可能有 OAuth 授权码等秘密（例如 /login/generic_oauth?code=…）。
        val page = Uri.parse(url).buildUpon().clearQuery().fragment(null).build()
        Log.i(TAG, "page finished loading, page=$page")
        controller.history.finished(url)
        controller.updateHistory()
        // 出错的加载也会走到这里。
        if (!failedThisLoad) {
            // 加载开始时网页框被藏起来，此时组合不一定再跑 update，按当前可见要求自己放出来。
            view.visibility = if ((view as ThemedPluginWebView).wantedVisible) View.VISIBLE else View.INVISIBLE
            onLoaded()
        }
    }
}
