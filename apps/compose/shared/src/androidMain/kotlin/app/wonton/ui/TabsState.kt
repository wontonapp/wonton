package app.wonton.ui

import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabRef

class TabsState(val plugins: PluginsState) {
    val core = plugins.core
    var viewModel by mutableStateOf(core.tabs())
        private set
    var pinRequest by mutableStateOf<PinRequest?>(null)
    /** 核心出了询问、还没答的那次网页请求；弹窗交回回答时用它放行或拒绝。 */
    var mediaWebRequest by mutableStateOf<android.webkit.PermissionRequest?>(null)
    /** 核心已放行、等系统运行时权限的那次网页请求。 */
    var mediaSystemRequest by mutableStateOf<android.webkit.PermissionRequest?>(null)

    suspend fun observe() {
        core.subscribeTabs().use { subscription ->
            while (true) viewModel = subscription.next() ?: break
        }
    }

    fun send(action: Action) = plugins.send(action)

    fun pin(tab: TabRef, title: String) {
        if (viewModel.full) pinRequest = PinRequest(tab, title) else send(Action.PinTab(tab, null))
    }
}

val LocalTabs = compositionLocalOf<TabsState> { error("TabsState is not provided") }

data class TabChoice(val tab: TabRef, val title: String)

val InstalledPlugin.tabChoices: List<TabChoice>
    get() = if (tabs.isNotEmpty()) {
        tabs.map { TabChoice(TabRef.Plugin(id, it.id), it.title) }
    } else if (webUrl != null) {
        listOf(TabChoice(TabRef.Plugin(id, null), title))
    } else {
        emptyList()
    }
