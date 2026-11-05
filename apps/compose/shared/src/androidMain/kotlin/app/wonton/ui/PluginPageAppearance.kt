package app.wonton.ui

import android.view.View
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.webkit.ScriptHandler
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat

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

/** 只提取主文档的显示属性；不接收操作，不参与 wontonBridge 的协议或权限。 */
internal class PluginPageAppearance(
    private val view: WebView,
    private val controller: PluginWebViewController,
) {
    private var origins: Set<String>? = null
    private var scriptHandler: ScriptHandler? = null
    private var navigation = 0L
    private var activated = false

    fun updateOrigins(next: Set<String>) {
        if (origins == next) return
        reset()
        removeHandlers()
        WebViewCompat.addWebMessageListener(view, LISTENER, next) { _, message, _, main, _ ->
            if (!main || !activated || controller.view !== view) return@addWebMessageListener
            if (message.type == WebMessageCompat.TYPE_STRING) message.data?.let(::receiveColor)
        }
        scriptHandler = WebViewCompat.addDocumentStartJavaScript(view, THEME_COLOR_SCRIPT, next)
        origins = next.toSet()
    }

    fun reset() {
        navigation++
        activated = false
        if (controller.view === view) controller.updateThemeColor(null)
    }

    fun activate() {
        if (activated || origins == null) return
        activated = true
        // 提交之后才给当前文档发代次；旧文档排队的消息不能给新页面染色。
        view.evaluateJavascript(
            "if (window.$LISTENER && window.$LISTENER.onmessage) { " +
                "window.$LISTENER.onmessage({data: '$navigation'}); }",
            null,
        )
    }

    fun dispose() {
        reset()
        removeHandlers()
    }

    private fun receiveColor(message: String) {
        val separator = message.indexOf(':')
        if (separator < 0 || message.substring(0, separator) != navigation.toString()) return
        val color = message.substring(separator + 1)
        if (color.isEmpty()) {
            controller.updateThemeColor(null)
        } else {
            color.toLongOrNull()?.takeIf { it in 0L..MAX_ARGB }?.let {
                controller.updateThemeColor(it.toInt())
            }
        }
    }

    private fun removeHandlers() {
        scriptHandler?.remove()
        scriptHandler = null
        if (origins != null) WebViewCompat.removeWebMessageListener(view, LISTENER)
        origins = null
    }
}

private const val LISTENER = "wontonPageAppearance"
private const val MAX_ARGB = 0xffffffffL

// 颜色交给 WebView 的 CSS 解析器和 sRGB canvas，不在 Kotlin 复制 CSS 颜色语法。
private val THEME_COLOR_SCRIPT = """
    (() => {
        if (window !== window.top || !window.wontonPageAppearance) return;
        const bridge = window.wontonPageAppearance;
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const context = canvas.getContext('2d', {willReadFrequently: true});
        const queries = new Map();
        let navigation = null;
        let previous;
        let observing = false;

        function parseColor(value) {
            value = value.trim();
            if (!context || !value || value.toLowerCase() === 'currentcolor') return null;
            context.fillStyle = '#010203';
            context.fillStyle = value;
            const first = context.fillStyle;
            context.fillStyle = '#040506';
            context.fillStyle = value;
            if (context.fillStyle !== first) return null;
            context.clearRect(0, 0, 1, 1);
            context.fillRect(0, 0, 1, 1);
            const rgba = context.getImageData(0, 0, 1, 1).data;
            return ((rgba[3] << 24) | (rgba[0] << 16) | (rgba[1] << 8) | rgba[2]) >>> 0;
        }

        function refresh() {
            const metas = document.querySelectorAll('meta[name="theme-color" i]');
            const required = new Set(['(prefers-color-scheme: dark)']);
            for (const meta of metas) {
                const media = (meta.getAttribute('media') || '').trim();
                if (media) required.add(media);
            }
            for (const [media, query] of queries) {
                if (!required.has(media)) {
                    query.removeEventListener('change', refresh);
                    queries.delete(media);
                }
            }
            for (const media of required) {
                if (!queries.has(media)) {
                    const query = matchMedia(media);
                    query.addEventListener('change', refresh);
                    queries.set(media, query);
                }
            }
            let color = null;
            for (const meta of metas) {
                const media = (meta.getAttribute('media') || '').trim();
                if (media && !queries.get(media).matches) continue;
                color = parseColor(meta.getAttribute('content') || '');
                if (color !== null) break;
            }
            const value = color === null ? '' : String(color);
            if (navigation !== null && value !== previous) {
                previous = value;
                bridge.postMessage(navigation + ':' + value);
            }
        }

        function containsMeta(node) {
            return node.nodeType === Node.ELEMENT_NODE &&
                (node.tagName === 'META' || node.querySelector('meta'));
        }

        const observer = new MutationObserver(records => {
            if (records.some(record => record.type === 'attributes'
                ? record.target.tagName === 'META'
                : [...record.addedNodes, ...record.removedNodes].some(containsMeta))) refresh();
        });

        function observe() {
            if (observing) return;
            observing = true;
            observer.observe(document, {
                subtree: true, childList: true, attributes: true,
                attributeFilter: ['name', 'content', 'media']
            });
            refresh();
        }

        bridge.onmessage = event => {
            navigation = event.data;
            previous = undefined;
            observe();
            refresh();
        };
        window.addEventListener('pageshow', observe);
        window.addEventListener('pagehide', () => {
            observing = false;
            observer.disconnect();
            for (const query of queries.values()) query.removeEventListener('change', refresh);
            queries.clear();
            navigation = null;
            previous = undefined;
        });
        observe();
    })();
""".trimIndent()
