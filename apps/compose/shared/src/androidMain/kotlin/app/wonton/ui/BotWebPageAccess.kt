package app.wonton.ui

import app.wonton.protocol.BotRef
import app.wonton.protocol.BotView
import app.wonton.protocol.BotWebFailure
import app.wonton.protocol.BotWebView
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabRef

internal data class BotWebPageAccess(
    val allowed: Boolean,
    val disclosure: Boolean,
    val isolated: Boolean,
)

/** NoWebui distinguishes an ordinary plugin page from unverified Bot metadata. */
internal fun botWebPageAccess(
    tab: TabRef, url: String?, plugin: InstalledPlugin?, bots: List<BotView>,
): BotWebPageAccess {
    val ordinary = BotWebPageAccess(true, false, plugin?.crossOriginIsolated == true)
    val identity = tab.botPageIdentity(plugin)
    val web = bots.firstOrNull { it.bot == identity }?.web
    val ordinaryPage = identity == null || web.isOrdinaryPluginPage(tab)
    return if (ordinaryPage) ordinary else readyBotPageAccess(identity, web, url)
}

private fun TabRef.botPageIdentity(plugin: InstalledPlugin?): BotRef? = when (this) {
    is TabRef.Bot -> bot
    is TabRef.Plugin, is TabRef.PluginPath -> plugin
        ?.takeIf { it.botUserId != null || it.chat != null }
        ?.let { BotRef(it.id, "default") }
    else -> null
}

private fun BotWebView?.isOrdinaryPluginPage(tab: TabRef): Boolean =
    tab !is TabRef.Bot && this is BotWebView.Unavailable && reason == BotWebFailure.NO_WEBUI

private fun readyBotPageAccess(identity: BotRef?, web: BotWebView?, url: String?): BotWebPageAccess {
    val ready = (web as? BotWebView.Ready)?.page?.takeIf { it.bot == identity }
        ?: return BotWebPageAccess(false, false, false)
    val sameOrigin = url != null && pluginOrigin(url) == pluginOrigin(ready.url)
    return BotWebPageAccess(sameOrigin, sameOrigin, ready.crossOriginIsolated)
}
