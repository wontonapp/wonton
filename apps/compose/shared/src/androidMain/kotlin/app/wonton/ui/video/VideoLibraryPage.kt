package app.wonton.ui.video

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.PagesState

/**
 * 插件页宿主里点开的影视库（`TabPage.native` 是影视）：返回和关掉走核心的页面栈，菜单里的网页版是同一插件的
 * `/` 页，详情交给宿主。
 */
@Composable
internal fun VideoLibraryPage(
    tab: TabRef.Plugin, title: String, pages: PagesState, active: Boolean, onDetails: (String) -> Unit,
) {
    val root = VideoRootActions(
        title,
        close = { pages.send(Action.Pages(PageAction.ClosePage)) },
        openWeb = { pages.send(Action.Pages(PageAction.OpenPage(TabRef.PluginPath(tab.pluginId, "/")))) },
        addToMyApps = { pages.addMyApp(MyAppRef.Page(tab)) },
        details = { onDetails(tab.pluginId) },
    )
    Box(Modifier.zIndex(if (active) 1f else 0f).fillMaxSize()) {
        VideoRoot(pages.core, tab.pluginId, root, active)
    }
}
