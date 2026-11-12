package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.webkit.WebViewFeature
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.ProxySettings
import app.wonton.protocol.SharedAppsSource
import app.wonton.protocol.SharedAppsState
import app.wonton.protocol.TabRef

internal data class SharedPageConnection(
    val shared: TabRef.SharedApp?,
    val source: SharedAppsSource?,
    val proxy: ProxySettings?,
    val profile: String?,
    val authorized: Boolean,
    val connected: Boolean,
    val supported: Boolean,
)

@Composable
internal fun sharedPageConnection(tab: TabRef, tabs: TabsState, home: HomeViewModel): SharedPageConnection {
    val shared = tab as? TabRef.SharedApp
    val source = shared?.let { selected -> tabs.plugins.viewModel.sharedApps?.sources
        ?.firstOrNull { it.ownerUserId == selected.ownerUserId } }
    val proxy = if (shared == null) home.proxy else (source?.state as? SharedAppsState.Ready)?.proxy
    val account = LocalPluginAccount.current
    val profile = if (shared != null && account != null) pluginProfileName(account, shared.ownerUserId) else null
    val row = source?.apps?.firstOrNull { it.plugin.pluginId == shared?.pluginId }
    val available = row != null && row.problem == null
    val authorized = if (shared == null) true else available && proxy != null && profile != null
    val connected = if (shared == null) home.status is HomeStatus.Connected else proxy != null
    return SharedPageConnection(shared, source, proxy, profile, authorized, connected, supportsWebView(shared != null))
}

private fun supportsWebView(shared: Boolean): Boolean {
    val supported = listOf(WebViewFeature.WEB_MESSAGE_LISTENER, WebViewFeature.DOCUMENT_START_SCRIPT,
        WebViewFeature.PROXY_OVERRIDE, WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS)
        .all(WebViewFeature::isFeatureSupported)
    return supported && (!shared || WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE))
}
