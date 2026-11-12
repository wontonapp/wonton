package app.wonton.ui

import android.net.Uri
import android.view.View
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
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

    override fun onPermissionRequest(request: PermissionRequest) {
        if (controller.view !== owner) {
            request.deny()
            return
        }
        controller.onPermissionRequest(request)
    }
    override fun onShowFileChooser(
        webView: WebView,
        filePathCallback: ValueCallback<Array<Uri>>,
        fileChooserParams: FileChooserParams,
    ): Boolean = controller.showFileChooser(owner, filePathCallback, fileChooserParams)
}
