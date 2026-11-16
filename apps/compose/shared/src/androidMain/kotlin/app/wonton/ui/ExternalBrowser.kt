package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent

/**
 * 外部网页插件（`runtime: link`，ADR-0080）用 Custom Tabs 打开，顶上显示网址，页面不给任何和 App 通信的桥。
 * 默认浏览器不支持 Custom Tabs 时同一个 Intent 由它直接打开。没有浏览器时如实报错，不退回 App 里的 WebView。
 */
internal fun openExternalBrowser(context: Context, url: String): Problem? {
    val tab = CustomTabsIntent.Builder().build()
    return try {
        tab.launchUrl(context, Uri.parse(url))
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
