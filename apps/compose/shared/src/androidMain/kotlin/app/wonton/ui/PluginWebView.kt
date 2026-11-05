package app.wonton.ui

import android.annotation.SuppressLint
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import android.view.ViewGroup
import android.webkit.HttpAuthHandler
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewFeature
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.ProxySettings
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.util.concurrent.Executor
import kotlinx.coroutines.delay

private const val TAG = "WontonWeb"

/** 和 `wonton-tunnel` 的 `ERROR_HEADER` 一致。 */
private const val TUNNEL_ERROR_HEADER = "x-wonton-tunnel-error"

/** 连上了还加载不了，试这么多次就照实说。 */
private const val MAX_ATTEMPTS = 3
private const val RETRY_DELAY_MS = 2_000L

/**
 * 插件网页。WebView 的 HTTP 代理设为核心的本地代理，`Host` 原样保留，插件地址和 OIDC 签发方地址都和
 * 服务器上一致。代理要求会话凭据：WebView 收到 407 时由 [ProxyAuthClient] 交出本次的凭据。
 *
 * [ProxyController] 对整个 App 的 WebView 生效。用反向绕过规则只让家里服务器的基础域名和它的子域名走代理，
 * CDN、字体等外部资源直连。不能再加 `removeImplicitRules()`：它对应的 `<-loopback>` 规则在反向模式下会把
 * `*.localhost` 排除出走代理的名单（实测这样 WebView 会直连 `*.wonton.localhost`）。
 */
/**
 * 全屏的插件网页，带一个关闭按钮。没连上时先打开，显示“连接中…”，连上后自动加载；加载中途断开也一样，本地代理的错误页不露出来。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PluginPage(title: String, url: String, home: HomeViewModel, takeover: PageTakeover? = null, onClose: () -> Unit) {
    val isConnected = home.status is HomeStatus.Connected
    var started by remember { mutableStateOf(false) }
    var failures by remember { mutableIntStateOf(0) }
    var reload by remember { mutableIntStateOf(0) }
    val gaveUp = isConnected && failures >= MAX_ATTEMPTS
    LaunchedEffect(isConnected) {
        if (isConnected) {
            started = true
            if (failures > 0) reload++
        }
    }
    LaunchedEffect(failures) {
        if (failures > 0 && !gaveUp) {
            delay(RETRY_DELAY_MS)
            if (home.status is HomeStatus.Connected) reload++
        }
    }
    val latestTakeover by rememberUpdatedState(takeover)
    DisposableEffect(url) { onDispose { latestTakeover?.end() } }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(home.notice?.sentence ?: title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                actions = {
                    takeover?.let { TakeoverButton(it) }
                    TextButton(onClick = onClose) { Text("关闭") }
                },
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            val proxy = home.proxy
            if (started && proxy != null) {
                PluginWebView(url, proxy, reload, onLoaded = { failures = 0 }, onFailed = { failures++ })
            }
            if (!started || failures > 0) {
                PageCover(gaveUp, onRetry = {
                    failures = 0
                    reload++
                })
            }
            takeover?.let { TakeoverNotice(it) }
        }
    }
}

@Composable
private fun PageCover(gaveUp: Boolean, onRetry: () -> Unit) {
    Column(
        Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12, Alignment.CenterVertically),
    ) {
        if (gaveUp) {
            Text(
                "网页打不开",
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            WontonSmallButton("重试", onRetry)
        } else {
            CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
        }
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun PluginWebView(
    url: String,
    proxy: ProxySettings,
    reload: Int,
    onLoaded: () -> Unit,
    onFailed: () -> Unit,
) {
    val loaded by rememberUpdatedState(onLoaded)
    val failed by rememberUpdatedState(onFailed)
    DisposableEffect(proxy) {
        overrideProxy(proxy)
        onDispose { }
    }
    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { context ->
            // 调试版才开：插件网页只在真机的旧 WebView 上出问题时，要靠 chrome://inspect 看。
            val debuggable = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
            WebView.setWebContentsDebuggingEnabled(debuggable)
            WebView(context).apply {
                // AndroidView 默认给 WRAP_CONTENT，WebView 因此把网页的视口高度算成 0，`height: 100%` 的页面
                // （比如 magpie）正文塌成一条，看上去是空白页（实测 Chrome 101 的 WebView）。
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                )
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                webViewClient = ProxyAuthClient(proxy, reload, { loaded() }, { failed() })
                loadUrl(url)
            }
        },
        update = { webView ->
            val client = webView.webViewClient as ProxyAuthClient
            if (client.proxy != proxy || client.reload != reload) {
                // 核心重建了本地代理时换上新凭据；重新加载时要回到 `url`，失败的那一页不算数。
                webView.webViewClient = ProxyAuthClient(proxy, reload, { loaded() }, { failed() })
                webView.loadUrl(url)
            }
        },
    )
}

private fun overrideProxy(proxy: ProxySettings) {
    check(
        WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS),
    ) {
        "这台设备的 WebView 太旧，不支持按域名设置代理，应用网页打不开"
    }
    val config = ProxyConfig.Builder()
        .addProxyRule("${proxy.host}:${proxy.port}")
        .addBypassRule(proxy.domain)
        .addBypassRule("*.${proxy.domain}")
        .setReverseBypassEnabled(true)
        .build()
    val sameThread = Executor { it.run() }
    ProxyController.getInstance().setProxyOverride(config, sameThread) {
        Log.i(TAG, "WebView 代理已设为 ${proxy.host}:${proxy.port}")
    }
}

/** 只把本地代理的凭据交给本地代理自己：别的主机（例如插件网页）要认证时不给。 */
private class ProxyAuthClient(
    val proxy: ProxySettings,
    val reload: Int,
    private val onLoaded: () -> Unit,
    private val onFailed: () -> Unit,
) : WebViewClient() {
    private var failedThisLoad = false

    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
        failedThisLoad = false
    }

    override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
        if (request.isForMainFrame) failed()
    }

    /** 本地代理连不上隧道时回的错误页不给主人看（ADR-0032）。插件自己的 502 不带这个响应头，照常显示。 */
    override fun onReceivedHttpError(view: WebView, request: WebResourceRequest, response: WebResourceResponse) {
        val headers = response.responseHeaders.orEmpty().keys
        val fromProxy = headers.any { it.equals(TUNNEL_ERROR_HEADER, ignoreCase = true) }
        if (request.isForMainFrame && fromProxy) failed()
    }

    private fun failed() {
        failedThisLoad = true
        onFailed()
    }

    override fun onReceivedHttpAuthRequest(view: WebView, handler: HttpAuthHandler, host: String, realm: String) {
        if (host == proxy.host || host == "${proxy.host}:${proxy.port}") {
            handler.proceed(proxy.username, proxy.password)
        } else {
            super.onReceivedHttpAuthRequest(view, handler, host, realm)
        }
    }

    override fun onPageFinished(view: WebView, url: String) {
        // 只记到路径：查询串和片段里可能有 OAuth 授权码等秘密（例如 /login/generic_oauth?code=…）。
        val page = Uri.parse(url).buildUpon().clearQuery().fragment(null).build()
        Log.i(TAG, "网页加载完成 page=$page title=${view.title}")
        // 出错的加载也会走到这里。
        if (!failedThisLoad) onLoaded()
    }
}
