package app.wonton.ui

import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabRef
import app.wonton.protocol.MyAppRef

/** 页面栈和“我的应用”的壳侧状态：核心算好 [`PagesViewModel`]，这里只发操作。 */
class PagesState(val plugins: PluginsState) {
    val core = plugins.core
    var viewModel by mutableStateOf(core.pages())
        private set
    /** 核心出了询问、还没答的那次网页请求；弹窗交回回答时用它放行或拒绝。 */
    var mediaWebRequest by mutableStateOf<android.webkit.PermissionRequest?>(null)
    /** 核心已放行、等系统运行时权限的那次网页请求。 */
    var mediaSystemRequest by mutableStateOf<android.webkit.PermissionRequest?>(null)

    suspend fun observe() {
        core.subscribePages().use { subscription ->
            while (true) viewModel = subscription.next() ?: break
        }
    }

    fun send(action: Action) = plugins.send(action)

    /** 核心当前显示的页面：应用网格、栈顶或根页面。 */
    val current: TabRef
        get() = if (viewModel.showsApps) TabRef.Apps else viewModel.stack.lastOrNull()?.tab ?: viewModel.root

    /** 把一个应用（插件整项或某一页）加进“我的应用”。重复加的错误交给调用方显示。 */
    fun addMyApp(app: MyAppRef, title: String? = null) = send(Action.Pages(PageAction.AddMyApp(app, title, null)))
}

val LocalPages = compositionLocalOf<PagesState> { error("PagesState is not provided") }

data class TabChoice(val tab: TabRef, val title: String)

val InstalledPlugin.tabChoices: List<TabChoice>
    get() = if (tabs.isNotEmpty()) {
        tabs.map { TabChoice(TabRef.Plugin(id, it.id), it.title) }
    } else if (webUrl != null) {
        listOf(TabChoice(TabRef.Plugin(id, null), title))
    } else {
        emptyList()
    }
