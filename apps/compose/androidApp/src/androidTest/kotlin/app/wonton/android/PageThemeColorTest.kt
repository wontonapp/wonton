@file:Suppress("MagicNumber") // 页面里写的颜色和取样的像素位置本来就是测试自定的。

package app.wonton.android

import android.Manifest
import android.os.Build
import android.webkit.WebView
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.ThemeMode
import app.wonton.ui.PageThemeColor
import app.wonton.ui.PageTopStrip
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 整页网页上面让出来的那一条跟页面最顶上实际显示的颜色（ADR-0095“铺满”）。假核心没有隧道，整页网页加载不出来，这里给一个真
 * WebView 装上核心的取色脚本和通道（[PageThemeColor]），加载一张本机写的网页，看顶条（[PageTopStrip]）画出来的颜色：
 * 顶栏的颜色（不是页面声明的那个浅色 `theme-color`）、半透明的一层盖上去后的合成色、顶上什么都没有时的背景色、
 * 背景也没有时声明的 `theme-color`（带 `media` 的按深浅色挑），都没有时的 App 底色。
 */
@RunWith(AndroidJUnit4::class)
class PageThemeColorTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()

    // 不先给通知权限的话，系统的授权框盖在上面，界面不在前台。
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    private var reported by mutableStateOf<Color?>(null)
    private var canvas = Color.Unspecified
    private lateinit var web: WebView

    @After
    fun release() {
        compose.runOnUiThread { if (::web.isInitialized) web.destroy() }
    }

    @Test
    fun stripFollowsWhatThePageShowsAtTheTop() {
        // 带 media 的那一条不匹配：结果不跟着设备的深浅色变。
        load(PAGE.replace("(prefers-color-scheme: dark)", "not all"))
        awaitStrip("theme-color 是浅色、固定的顶栏是深色：跟顶栏", Color(0xFF1F3A5F))

        run(
            "document.body.insertAdjacentHTML('beforeend', '<div id=veil style=\"position:fixed;top:0;left:0;right:0;" +
                "height:48px;background:rgba(255,255,255,0.5)\"></div>')",
        )
        awaitStrip("半透明的一层盖在顶栏上：合成出来的颜色", Color(0xFF8F9DAF))

        run("document.getElementById('veil').remove(); document.getElementById('bar').remove()")
        awaitStrip("顶上没有东西时是 body 的背景色", Color(0xFF17324D))

        run("document.body.style.background = 'transparent'")
        awaitStrip("body 透明时是 html 的背景色", Color(0xFF0B132B))

        run("document.documentElement.style.background = 'transparent'")
        awaitStrip("顶上什么都没涂时用声明的 theme-color", Color(0xFF90CAF9))

        run("document.querySelectorAll('meta[name=theme-color]').forEach(meta => meta.remove())")
        awaitStrip("页面没有可用的颜色时是 App 底色", canvas)
    }

    @Test
    fun declaredColorIsPickedByMedia() {
        load(BARE.replace("(prefers-color-scheme: dark)", "all"))
        awaitStrip("media 匹配的那一条 theme-color", Color(0xFF1B4965))
    }

    @Test
    fun invalidDeclarationIsNotUsed() {
        val invalid = BARE.replace("#90caf9", "not-a-color").replace("#1b4965", "rgba(0, 0, 0, 0.5)")
        // 先等页面出来，免得把加载前的 App 底色当成结果。
        load(invalid.replace("<body>", "<body style=\"background: #17324d\">"))
        awaitStrip("页面出来了", Color(0xFF17324D))
        run("document.body.style.background = 'transparent'")
        awaitStrip("声明写错或半透明时不用它，是 App 底色", canvas)
    }

    private fun load(html: String) {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val appearance = core.appearance().copy(themeMode = ThemeMode.LIGHT)
        canvas = appearance.light.canvas.toColor()
        compose.runOnUiThread {
            web = WebView(compose.activity).apply { settings.javaScriptEnabled = true }
            PageThemeColor(web) { reported = it }.update(setOf(ORIGIN))
            compose.activity.setContent {
                WontonTheme(appearance) {
                    // 网页要真的排在屏幕上：取色脚本按视口顶上的一点找元素，没有大小的 WebView 里什么都点不到。
                    Column {
                        PageTopStrip(reported, 96.dp)
                        AndroidView(factory = { web }, modifier = Modifier.fillMaxSize())
                    }
                }
            }
            web.loadDataWithBaseURL("$ORIGIN/", html, "text/html", "utf-8", null)
        }
    }

    private fun run(script: String) = compose.runOnUiThread { web.evaluateJavascript(script, null) }

    private fun awaitStrip(what: String, expected: Color) {
        var seen = Color.Unspecified
        val waited = runCatching {
            compose.waitUntil(WAIT_MS) {
                seen = stripColor()
                seen == expected
            }
        }
        check(waited.isSuccess) {
            "$what：顶条是 $seen，应该是 $expected（页面报的 $reported；${waited.exceptionOrNull()}）"
        }
    }

    /** 屏幕上顶条下沿往上一点、水平正中的像素：顶条从屏幕顶开始，上半截压着状态栏的图标。整屏截图才带得上系统栏那一截。 */
    private fun stripColor(): Color {
        val bounds = compose.onNodeWithTag("page-top-strip").fetchSemanticsNode().boundsInWindow
        val screen = requireNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
        try {
            return Color(screen.getPixel(bounds.center.x.toInt(), bounds.bottom.toInt() - 4))
        } finally {
            screen.recycle()
        }
    }

    private companion object {
        const val WAIT_MS = 10_000L
        const val ORIGIN = "https://corner.example"
        const val HEAD = """<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#1b4965">
            <meta name="theme-color" content="#90caf9">"""
        val PAGE = """
            <!doctype html>
            <html style="background: #0b132b"><head>
            $HEAD
            </head><body style="margin: 0; background: #17324d">
            <div id="bar" style="position: fixed; top: 0; left: 0; right: 0; height: 48px; background: #1f3a5f">Fixture</div>
            </body></html>
        """.trimIndent()
        val BARE = """
            <!doctype html>
            <html><head>
            $HEAD
            </head><body>Fixture</body></html>
        """.trimIndent()
    }
}
