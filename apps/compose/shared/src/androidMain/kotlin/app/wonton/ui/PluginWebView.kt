package app.wonton.ui

import android.annotation.SuppressLint
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.content.res.Configuration
import android.net.Uri
import android.util.Log
import android.view.View
import android.view.MotionEvent
import android.view.ViewGroup
import android.view.ContextThemeWrapper
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.HttpAuthHandler
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
    internal var fileChooserLauncher: ActivityResultLauncher<Intent>? = null
    internal var pendingFileChooser: ValueCallback<Array<Uri>>? = null
    var canGoBack by mutableStateOf(false)
        private set
    var customView: View? by mutableStateOf(null)
        private set
    private var customViewCallback: WebChromeClient.CustomViewCallback? = null

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

/** 本页该用的 Profile；来源不在本机代理里、或需要 Profile 而系统不支持时返回 null。 */
@Composable
private fun rememberWebLayer(proxy: ProxySettings?, profileName: String?): WebLayer? {
    val route = LocalPluginProxy.current
    val account = LocalPluginAccount.current
    val profile = profileName ?: account?.let { pluginProfileName(it) }
    val multiProfile = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)
    val onRoute = proxy != null && route?.contains(proxy) == true
    val unsupported = profileName != null && !multiProfile
    return if (!onRoute || unsupported) {
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
    val layer = rememberWebLayer(proxy, profileName) ?: return
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
                    if (layer.multiProfile && layer.profile != null) {
                        WebViewCompat.setProfile(this, layer.profile)
                    } else if (publicPage) WebViewCompat.setProfile(this, publicProfile.getName())
                    configureAppearance(this, canvas, darkTheme)
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
                if (!publicPage) {
                    view.updateBridge(allowedOrigins, documentStartScript) { origin, main, message ->
                        bridge(origin, main, message)
                    }
                }
                configureBackground(view, canvas)
                view.updateTheme(darkTheme, canvas)
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

private class ThemedPluginWebView(
    private val wrapped: ContextThemeWrapper,
    private var dark: Boolean,
    private val controller: PluginWebViewController,
) : WebView(wrapped) {
    var rendererGone = false
    private var disposed = false
    private var bridgeOrigins: Set<String>? = null
    private var startScript: String? = null
    private var scriptHandler: androidx.webkit.ScriptHandler? = null
    private val pageMedia = PageMediaPage(this)

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        controller.touchObserver?.invoke(event)
        return super.dispatchTouchEvent(event)
    }

    fun disposePage() {
        if (disposed) return
        disposed = true
        scriptHandler?.remove()
        scriptHandler = null
        pageMedia.dispose()
        if (bridgeOrigins != null) WebViewCompat.removeWebMessageListener(this, "wontonBridge")
        bridgeOrigins = null
        controller.release(this)
        webChromeClient = null
    }

    fun updateBridge(origins: Set<String>, script: String, onMessage: (Uri, Boolean, String) -> Unit) {
        require(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER))
        require(WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT))
        pageMedia.update(origins)
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
        controller.updateHistory()
        // 出错的加载也会走到这里。
        if (!failedThisLoad) {
            onLoaded()
        }
    }
}
