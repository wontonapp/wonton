package app.wonton.ui

import android.view.View
import android.webkit.WebChromeClient
import android.webkit.WebView

internal class PluginChromeClient(
    private val owner: WebView,
    private val controller: PluginWebViewController,
) : WebChromeClient() {
    override fun onShowCustomView(view: View, callback: CustomViewCallback) {
        controller.showCustomView(owner, view, callback)
    }

    override fun onHideCustomView() {
        if (controller.view === owner) controller.closeCustomView()
    }
}
