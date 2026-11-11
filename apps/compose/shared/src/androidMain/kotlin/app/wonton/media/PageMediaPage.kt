package app.wonton.media

import android.webkit.WebView
import androidx.webkit.ScriptHandler
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import app.wonton.client.ClientCore

interface PageMediaCoreOwner {
    val core: Result<ClientCore>
}

class PageMediaPage(private val view: WebView) {
    private val core = (view.context.applicationContext as PageMediaCoreOwner).core.getOrThrow()
    private var origins: Set<String>? = null
    private var script: ScriptHandler? = null

    fun update(nextOrigins: Set<String>) {
        if (origins == nextOrigins) return
        removeChannel()
        WebViewCompat.addWebMessageListener(view, "wontonMedia", nextOrigins) { _, message, _, main, _ ->
            if (main && message.type == WebMessageCompat.TYPE_STRING) {
                val state = message.data?.let { core.pageMediaMessage(it) }
                if (state != null) PageMediaOwner.update(view, core, state)
            }
        }
        script = WebViewCompat.addDocumentStartJavaScript(view, core.pageMediaScript(), nextOrigins)
        origins = nextOrigins.toSet()
    }

    fun dispose() {
        removeChannel()
        PageMediaOwner.dispose(view)
    }

    private fun removeChannel() {
        script?.remove()
        script = null
        if (origins != null) WebViewCompat.removeWebMessageListener(view, "wontonMedia")
        origins = null
    }
}
