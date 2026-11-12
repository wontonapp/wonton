package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

internal fun TabRef.localPluginId(): String? = when (this) {
    is TabRef.Plugin -> pluginId
    is TabRef.PluginPath -> pluginId
    is TabRef.Bot -> bot.pluginId
    else -> null
}

internal fun TabsState.installedPlugin(tab: TabRef): InstalledPlugin? {
    val id = tab.localPluginId() ?: return null
    return plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id }
}

/**
 * Chrome 认的 Custom Tabs 附加项：页面开着时浏览器绑定 App 里的这个空服务，App 的进程跟着浏览器保持前台的优先级，
 * 不被冻结，回环入口和隧道才答得上浏览器的请求。不认它的浏览器照样打开，只是 App 可能被挂起。
 */
private const val EXTRA_KEEP_ALIVE = "android.support.customtabs.extra.KEEP_ALIVE"
private const val KEEP_ALIVE_SERVICE = "app.wonton.android.browser.BrowserKeepAliveService"

/**
 * 要跨源隔离的插件页（`TabPage.opensInBrowser`，ADR-0033）：Android WebView 给不了，App 里不加载，只留一行说明。
 * [autoOpen] 时进来就交给浏览器一次；回到 App 再点按钮重新打开。
 */
@Composable
internal fun BrowserPageStatus(page: TabPage, core: ClientCore, autoOpen: Boolean) {
    val context = LocalContext.current
    val url = page.url ?: return
    var problem by remember(url) { mutableStateOf<Problem?>(null) }
    var opened by rememberSaveable(url, page.reload) { mutableStateOf(false) }
    val open = { problem = openPluginInBrowser(context, core, url) }
    LaunchedEffect(url, page.reload, autoOpen) {
        if (autoOpen && !opened) {
            opened = true
            open()
        }
    }
    problem?.let {
        ProblemView(it, onRetry = open)
        return
    }
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.tab_opens_in_browser, page.title),
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonButton(stringResource(R.string.tab_open_in_browser), open)
    }
}

/**
 * 换成核心给的浏览器地址，用 Custom Tabs 打开；默认浏览器不支持 Custom Tabs 时同一个 Intent 由它直接打开。
 * 没有浏览器时如实报错，不退回 App 里打不开的 WebView。
 */
internal fun openPluginInBrowser(context: Context, core: ClientCore, url: String): Problem? {
    val target = try {
        core.pluginBrowserUrl(url)
    } catch (error: CoreException) {
        return error.problem
    }
    val tab = CustomTabsIntent.Builder().build()
    tab.intent.putExtra(EXTRA_KEEP_ALIVE, Intent().setComponent(ComponentName(context.packageName, KEEP_ALIVE_SERVICE)))
    return try {
        tab.launchUrl(context, Uri.parse(target))
        null
    } catch (failure: ActivityNotFoundException) {
        // 详情不放 Intent 原文：里面有浏览器地址的标签。
        Problem(
            titleRes = R.string.tab_no_browser,
            nextStepRes = R.string.tab_no_browser_next_step,
            detail = failure.javaClass.simpleName,
        )
    }
}
