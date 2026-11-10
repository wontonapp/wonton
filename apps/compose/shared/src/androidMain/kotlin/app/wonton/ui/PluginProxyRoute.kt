package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewFeature
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.ProxySettings
import app.wonton.protocol.SharedAppsState
import java.security.MessageDigest
import java.util.concurrent.Executor

internal data class PluginProxyRoute(val host: String, val port: UShort, val domains: Set<String>) {
    fun contains(proxy: ProxySettings): Boolean =
        host == proxy.host && port == proxy.port && proxy.domain in domains
}

internal val LocalPluginProxy = compositionLocalOf<PluginProxyRoute?> { null }
internal val LocalPluginAccount = compositionLocalOf<String?> { null }

/** ProxyController is process-global: configure the hub and all active domains together, never per page. */
@Composable
internal fun pluginProxyRoute(home: HomeViewModel, plugins: PluginsState): PluginProxyRoute? {
    val proxies = listOfNotNull(home.proxy) + plugins.viewModel.sharedApps?.sources.orEmpty().mapNotNull {
        (it.state as? SharedAppsState.Ready)?.proxy
    }
    val endpoint = proxies.firstOrNull()
    val route = endpoint?.takeIf { first -> proxies.all { it.host == first.host && it.port == first.port } }?.let {
        PluginProxyRoute(it.host, it.port, proxies.map { proxy -> proxy.domain }.toSet())
    }
    var ready by remember { mutableStateOf<PluginProxyRoute?>(null) }
    DisposableEffect(route) {
        var active = true
        ready = null
        if (route != null && WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS)) {
            val builder = ProxyConfig.Builder().addProxyRule("${route.host}:${route.port}")
            route.domains.forEach { domain -> builder.addBypassRule(domain).addBypassRule("*.$domain") }
            ProxyController.getInstance().setProxyOverride(builder.setReverseBypassEnabled(true).build(),
                Executor { it.run() }) { if (active) ready = route }
        }
        onDispose { active = false }
    }
    return ready.takeIf { it == route }
}

internal fun pluginProfileName(account: String, owner: String? = null): String {
    val identity = "$account\u0000${owner.orEmpty()}"
    return "wonton-" + MessageDigest.getInstance("SHA-256").digest(identity.toByteArray())
        .joinToString("") { "%02x".format(it) }
}
