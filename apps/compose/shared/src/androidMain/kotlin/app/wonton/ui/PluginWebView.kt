package app.wonton.ui

import android.annotation.SuppressLint
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.util.Log
import android.view.ViewGroup
import android.webkit.HttpAuthHandler
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewFeature
import app.wonton.protocol.ProxySettings
import java.util.concurrent.Executor

private const val TAG = "WontonWeb"

/**
 * 插件网页。WebView 的 HTTP 代理设为核心的本地代理，`Host` 原样保留，插件地址和 OIDC 签发方地址都和
 * 服务器上一致。代理要求会话凭据：WebView 收到 407 时由 [ProxyAuthClient] 交出本次的凭据。
 *
 * [ProxyController] 对整个 App 的 WebView 生效。用反向绕过规则只让家里服务器的基础域名和它的子域名走代理，
 * CDN、字体等外部资源直连。不能再加 `removeImplicitRules()`：它对应的 `<-loopback>` 规则在反向模式下会把
 * `*.localhost` 排除出走代理的名单（实测这样 WebView 会直连 `*.wonton.localhost`）。
 */
/** 全屏的插件网页，带一个关闭按钮。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PluginPage(title: String, url: String, proxy: ProxySettings, onClose: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                actions = { TextButton(onClick = onClose) { Text("关闭") } },
            )
        },
    ) { padding ->
        PluginWebView(url, proxy, Modifier.padding(padding).fillMaxSize())
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun PluginWebView(url: String, proxy: ProxySettings, modifier: Modifier = Modifier) {
    DisposableEffect(proxy) {
        overrideProxy(proxy)
        onDispose { }
    }
    AndroidView(
        modifier = modifier,
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
                webViewClient = ProxyAuthClient(proxy)
                loadUrl(url)
            }
        },
        update = { webView ->
            val client = webView.webViewClient as ProxyAuthClient
            if (client.proxy != proxy) {
                // 核心重建了本地代理：换上新凭据，重新加载。
                webView.webViewClient = ProxyAuthClient(proxy)
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
private class ProxyAuthClient(val proxy: ProxySettings) : WebViewClient() {
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
    }
}
