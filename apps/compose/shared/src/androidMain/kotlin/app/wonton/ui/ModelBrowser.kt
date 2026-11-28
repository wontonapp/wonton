package app.wonton.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.LocalActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.ModelLoginBrowser
import app.wonton.protocol.ModelLoginProgress
import app.wonton.protocol.ModelProviderPage

/**
 * 模型页用的 App 内浏览器（Custom Tabs）：去拿 key、设备码登录和浏览器授权都从这里开。
 * 授权的回跳（`wonton://model-login`）由 Activity 收下直接交给核心，不经过这里。
 */
class ModelBrowser(private val context: Context, private val reject: (Problem) -> Unit) {
    /** 为这次登录开过浏览器的网址；登录结束时清掉。 */
    var loginUrl by mutableStateOf<String?>(null)
        private set

    fun open(url: String): Boolean {
        val problem = openExternalBrowser(context, url)
        problem?.let(reject)
        return problem == null
    }

    fun openLogin(login: ModelLoginProgress) {
        if (open(login.url)) loginUrl = login.url
    }

    fun loginEnded() {
        loginUrl = null
    }
}

/**
 * 浏览器回来的地址是不是模型登录的回跳。地址是协议里的 `MODEL_LOGIN_REDIRECT_URI`（`wonton://model-login`），
 * 核心拿它拼授权网址；它没有经绑定导出，这里照写一份。
 */
fun isModelLoginCallback(url: String): Boolean {
    val uri = Uri.parse(url)
    return uri.scheme.equals("wonton", ignoreCase = true) && uri.host == "model-login"
}

/**
 * 登录一开始就自己打开浏览器的：浏览器授权，和码没带在网址里的设备码登录（码在网址里时浏览器会替人填好）。
 * 码要人自己抄的，先在页面上把码亮出来，人点了才开。
 */
internal fun ModelLoginProgress.opensByItself(): Boolean = when (browser) {
    ModelLoginBrowser.REDIRECT -> true
    ModelLoginBrowser.PAGE -> code.isNullOrEmpty() || url.contains(code.orEmpty())
    ModelLoginBrowser.OTHER -> false
}

/**
 * 按登录的进展开关浏览器（ADR-0129 第 3 节）：
 * - 登录开始：该自己开的就开。
 * - 核心清掉了登录、也没有失败的结果（批准了）：把自己的页面拉回前台，盖住浏览器。
 * - 人自己关了浏览器回来、登录还在等：算取消。码要抄的那种不算——人回来多半是为了看码，取消留给页面上的按钮。
 *   自己的 Activity 只有在盖着它的 Custom Tabs 关掉时才会 resume，切去别的 App 再回来看到的还是浏览器。
 */
@Composable
internal fun ModelLoginBrowserEffect(models: AgentModels) {
    val page = models.view.provider
    val login = page?.login
    val browser = models.browser
    val activity = LocalActivity.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(login?.url) {
        if (login != null && login.opensByItself() && browser.loginUrl != login.url) browser.openLogin(login)
    }
    val approved = page != null && page.loginOutcome == null && models.view.actionProblem == null
    LaunchedEffect(login == null) {
        if (login != null || browser.loginUrl == null) return@LaunchedEffect
        browser.loginEnded()
        if (approved && !lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) activity?.bringToFront()
    }
    val latest by rememberUpdatedState(models.latest)
    val send by rememberUpdatedState(models.send)
    LifecycleResumeEffect(browser) {
        // 组合里的 ViewModel 这时还是离开前的：回跳刚交给核心的那一下要问核心此刻的值。
        if (latest().provider?.leftLoginBrowser(browser.loginUrl) == true) {
            browser.loginEnded()
            send(AgentModelsAction.CancelModelLogin)
        }
        onPauseOrDispose { }
    }
}

/** 为这次登录自己开的浏览器已经不在前面了，登录却还在等（回跳也没交回来）。 */
private fun ModelProviderPage.leftLoginBrowser(loginUrl: String?): Boolean {
    val pending = login?.takeIf { it.url == loginUrl && it.opensByItself() }
    return pending != null && !busy
}

/** 清掉盖在上面的 Custom Tabs，回到自己这一页；不新建 Activity。 */
private fun Activity.bringToFront() {
    startActivity(
        Intent(this, javaClass).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
    )
}
