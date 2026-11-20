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
import app.wonton.media.PageMediaCoreOwner
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private val reportedColor = Regex("^#[0-9a-f]{6}$")

/** 页面脚本报上来的 `#rrggbb`；别的一律当没有（页面能往这个通道里发任何东西）。 */
internal fun parsePageColor(message: String): Color? {
    if (!reportedColor.matches(message)) return null
    return Color(message.toColorInt())
}

/**
 * 给一个 WebView 装上核心的取色脚本（`ClientCore.pageColorScript`，两端共用，读法见核心的 `pages/color/script.js`）和回报通道：
 * 页面最顶上实际显示的颜色。[onColor] 在主线程上收到 null 表示页面没有可用的颜色。
 */
class PageThemeColor(private val view: WebView, private val onColor: (Color?) -> Unit) {
    private val core = (view.context.applicationContext as PageMediaCoreOwner).core.getOrThrow()
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
        script = WebViewCompat.addDocumentStartJavaScript(view, core.pageColorScript(), nextOrigins)
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
 * 整页网页上面让出来的那一条（状态栏，有胶囊的页再加胶囊那一行，ADR-0095“铺满”）：涂页面自己的颜色，和页面的顶栏连成一片；
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
