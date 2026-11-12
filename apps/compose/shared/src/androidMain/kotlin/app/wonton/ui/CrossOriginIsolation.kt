package app.wonton.ui

import androidx.compose.runtime.mutableStateMapOf
import androidx.webkit.Profile
import androidx.webkit.ProfileStore
import androidx.webkit.WebViewFeature
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabRef

/** 只存内存：主人在本次 App 进程里选择“仍然打开”后，不再重复拦截这个插件。 */
private val openedAnyway = mutableStateMapOf<String, Boolean>()
private val crossOriginIsolationSupported by lazy {
    WebViewFeature.isFeatureSupported(WebViewFeature.CROSS_ORIGIN_ISOLATED_ALLOWLIST)
}

internal fun TabsState.installedPlugin(tab: TabRef): InstalledPlugin? {
    val id = (tab as? TabRef.Plugin)?.pluginId ?: return null
    return plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id }
}

/** 先配置平台，再把已装列表发布给 UI；平台更新也作用于已加载的页面。 */
internal fun configureCrossOriginIsolatedAllowlist(
    plugins: List<InstalledPlugin>, profile: String = Profile.DEFAULT_PROFILE_NAME,
) {
    if (!crossOriginIsolationSupported) return
    val origins = buildSet {
        for (plugin in plugins) {
            val url = plugin.webUrl
            if (plugin.crossOriginIsolated && url != null) {
                val origin = pluginOrigin(url)
                require('*' !in origin) { "cross-origin isolation only allows exact origins" }
                add(origin)
            }
        }
    }
    ProfileStore.getInstance().getOrCreateProfile(profile)
        .setCrossOriginIsolatedAllowlist(origins)
}

internal fun isCrossOriginIsolationBlocked(plugin: InstalledPlugin?): Boolean =
    plugin != null && plugin.crossOriginIsolated && !crossOriginIsolationSupported && openedAnyway[plugin.id] != true

internal fun allowCrossOriginIsolationAnyway(pluginId: String) {
    openedAnyway[pluginId] = true
}
