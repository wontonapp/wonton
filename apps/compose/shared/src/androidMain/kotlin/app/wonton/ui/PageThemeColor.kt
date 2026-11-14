package app.wonton.ui

import android.webkit.WebView
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.core.graphics.toColorInt
import androidx.core.view.WindowCompat
import androidx.webkit.ScriptHandler
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 页面想让浏览器外框用的颜色，照 Chrome 和 Safari 的读法：`<meta name="theme-color">`（带 `media` 的按当前深浅色挑），
 * 没声明就用 `body`、再 `html` 的背景色，都没有就不报。单页应用切路由、切深浅色时跟着重报。
 * 任何 CSS 颜色写法都画成一个像素再读回来，半透明和写错的不算。
 *
 * Android 的 WebView 没有读 theme-color 的接口（iOS 有 `WKWebView.themeColor`），只能由页面脚本读 DOM。
 * 脚本只在页面改了 `<head>`、`html`/`body` 的属性、样式表加载完（捕获阶段的 `load`）、深浅色变了的时候重算，不盯整棵 DOM；
 * 重算用 `setTimeout` 合并，不用 `requestAnimationFrame`：加载中的 WebView 是隐藏的，那时候它不触发。
 */
private val pageColorScript = """
    (() => {
      const channel = window.wontonPageColor;
      if (window !== window.top || !channel) return;
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      const opaque = value => {
        if (!context || !value || !CSS.supports('color', value)) return null;
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = value;
        context.fillRect(0, 0, 1, 1);
        const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
        if (alpha !== 255) return null;
        return '#' + [red, green, blue].map(part => part.toString(16).padStart(2, '0')).join('');
      };
      const declared = () => {
        for (const meta of document.querySelectorAll('meta[name="theme-color" i]')) {
          const media = meta.getAttribute('media');
          if (media && !matchMedia(media).matches) continue;
          const color = opaque(meta.getAttribute('content'));
          if (color) return color;
        }
        return null;
      };
      const painted = () => {
        for (const element of [document.body, document.documentElement]) {
          const color = element && opaque(getComputedStyle(element).backgroundColor);
          if (color) return color;
        }
        return null;
      };
      let last;
      let queued = false;
      const report = () => {
        queued = false;
        let color = '';
        try { color = declared() || painted() || ''; } catch (_) { color = ''; }
        if (color === last) return;
        last = color;
        channel.postMessage(color);
      };
      const schedule = () => {
        if (queued) return;
        queued = true;
        setTimeout(report, 50);
      };
      const observer = new MutationObserver(schedule);
      const watch = () => {
        observer.disconnect();
        if (document.head) {
          observer.observe(document.head, {
            childList: true, subtree: true, characterData: true,
            attributes: true, attributeFilter: ['name', 'content', 'media'],
          });
        }
        for (const element of [document.documentElement, document.body]) {
          if (element) observer.observe(element, { attributes: true });
        }
        schedule();
      };
      document.addEventListener('DOMContentLoaded', watch);
      addEventListener('load', watch);
      addEventListener('pageshow', schedule);
      document.addEventListener('load', schedule, true);
      matchMedia('(prefers-color-scheme: dark)').addEventListener('change', schedule);
      schedule();
    })();
""".trimIndent()

private val reportedColor = Regex("^#[0-9a-f]{6}$")

/** 页面脚本报上来的 `#rrggbb`；别的一律当没有（页面能往这个通道里发任何东西）。 */
internal fun parsePageColor(message: String): Color? {
    if (!reportedColor.matches(message)) return null
    return Color(message.toColorInt())
}

/** 给一个 WebView 装上读页面颜色的脚本和回报通道；[onColor] 在主线程上收到 null 表示页面没有可用的颜色。 */
class PageThemeColor(private val view: WebView, private val onColor: (Color?) -> Unit) {
    private var origins: Set<String>? = null
    private var script: ScriptHandler? = null

    fun update(nextOrigins: Set<String>) {
        if (origins == nextOrigins) return
        removeChannel()
        WebViewCompat.addWebMessageListener(view, CHANNEL, nextOrigins) { _, message, _, main, _ ->
            if (main && message.type == WebMessageCompat.TYPE_STRING) {
                onColor(message.data?.let(::parsePageColor))
            }
        }
        script = WebViewCompat.addDocumentStartJavaScript(view, pageColorScript, nextOrigins)
        origins = nextOrigins.toSet()
    }

    fun dispose() {
        removeChannel()
        onColor(null)
    }

    private fun removeChannel() {
        script?.remove()
        script = null
        if (origins != null) WebViewCompat.removeWebMessageListener(view, CHANNEL)
        origins = null
    }

    private companion object {
        const val CHANNEL = "wontonPageColor"
    }
}

/**
 * 整页网页上面让出来的那一条（状态栏加胶囊那一行，ADR-0095“铺满”）：涂页面自己的颜色，和页面的顶栏连成一片；
 * 页面没有可用的颜色、还在加载或出错时是 App 底色。状态栏的图标按这条的深浅换，不然深色 App 里开浅色页面时白字压白底。
 */
@Composable
fun PageTopStrip(color: Color?, height: Dp, modifier: Modifier = Modifier) {
    Box(
        modifier.fillMaxWidth().height(height)
            .background(color ?: WontonTheme.colors.canvas.toColor())
            .testTag("page-top-strip"),
    )
    if (color != null) StatusBarIconsOn(color)
}

/** 状态栏图标在 [background] 上看得清；离开时还给 App 的深浅色（`WontonTheme` 的 `SystemBarsFollow`）。 */
@Composable
private fun StatusBarIconsOn(background: Color) {
    val view = LocalView.current
    val window = LocalActivity.current?.window ?: return
    val appDark = WontonTheme.isDark
    val darkIcons = background.luminance() > LIGHT_BACKGROUND
    DisposableEffect(window, view, darkIcons, appDark) {
        val bars = WindowCompat.getInsetsController(window, view)
        bars.isAppearanceLightStatusBars = darkIcons
        onDispose { bars.isAppearanceLightStatusBars = !appDark }
    }
}

/** 相对亮度过了这条线算浅色背景，用深色图标（W3C 对比度公式下黑字和白字对比度相等的点约 0.18）。 */
private const val LIGHT_BACKGROUND = 0.18f
