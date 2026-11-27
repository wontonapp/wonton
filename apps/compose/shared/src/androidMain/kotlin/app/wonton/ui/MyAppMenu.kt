package app.wonton.ui

import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.PageAction
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.SidebarApp
import app.wonton.protocol.SidebarOpen
import app.wonton.protocol.TabRef
import app.wonton.protocol.TakeoverState

/**
 * “我的应用”一行的长按菜单要用的东西。整页应用上没有壳的菜单（ADR-0095），针对这一页的操作都收在这里。
 * [tab] 是点这一行会在 App 里打开的网页；交给系统浏览器的行和 Bot 私聊行没有。
 */
internal class MyAppMenu(
    val app: SidebarApp, val plugin: InstalledPlugin?, inBrowser: Boolean,
    val pages: PagesState, val onGo: (SidebarDestination) -> Unit,
) {
    val tab: TabRef? = (app.open as? SidebarOpen.Page)?.tab?.takeIf { !inBrowser }

    val hasItems: Boolean get() = tab != null || plugin != null

    fun open(tab: TabRef) = onGo(SidebarDestination.Plugin(PluginEntry.Web(tab)))
}

@Composable
internal fun MyAppMenuItems(menu: MyAppMenu, onDismiss: () -> Unit) {
    menu.tab?.let { tab ->
        ReloadItem(menu, tab, onDismiss)
        if (menu.plugin?.declaresTools == true) TakeoverItem(menu, menu.plugin, tab, onDismiss)
    }
    menu.plugin?.let { plugin ->
        MembershipItems(plugin, menu.pages, onDismiss)
        DropdownMenuItem(
            text = { Text(stringResource(R.string.tab_menu_details)) },
            onClick = { onDismiss(); menu.onGo(SidebarDestination.Details(plugin.id)) },
        )
    }
}

/**
 * 重新加载：先把页打开再让核心重载。关掉的页还留着 WebView（[PluginPagesHost]），只打开不会重新请求；
 * 核心只认栈里的页，没打开成（插件停着）就不发，免得多报一条“不能重新加载”。
 */
@Composable
private fun ReloadItem(menu: MyAppMenu, tab: TabRef, onDismiss: () -> Unit) {
    DropdownMenuItem(
        text = { Text(stringResource(R.string.tab_menu_reload)) },
        onClick = {
            onDismiss()
            menu.open(tab)
            val pages = menu.pages
            if (pages.core.pages().stack.any { it.tab == tab }) {
                pages.send(Action.Pages(PageAction.ReloadTab(PlacedTab.Page(tab))))
            }
        },
    )
}

/** “我来操作”（ADR-0024）：接管的是页面上的操作，开始时把页打开；离开那一页时由页面自己交还（[pageTakeover]）。 */
@Composable
private fun TakeoverItem(menu: MyAppMenu, plugin: InstalledPlugin, tab: TabRef, onDismiss: () -> Unit) {
    val takeover = PageTakeover(plugin.id, menu.pages.plugins.viewModel.takeover, menu.pages::send)
    when (takeover.state) {
        is TakeoverState.Operating -> DropdownMenuItem(
            text = { Text(stringResource(R.string.takeover_hand_back)) },
            onClick = { onDismiss(); takeover.end() },
        )
        is TakeoverState.Starting -> DropdownMenuItem(
            text = { Text(stringResource(R.string.takeover_start)) }, onClick = {}, enabled = false,
        )
        else -> DropdownMenuItem(
            text = { Text(stringResource(R.string.takeover_start)) },
            onClick = { onDismiss(); menu.open(tab); takeover.start() },
        )
    }
}

/** 每个页面一项，没加进的加进“我的应用”，加进了的移出。 */
@Composable
private fun MembershipItems(plugin: InstalledPlugin, pages: PagesState, onDismiss: () -> Unit) {
    plugin.tabChoices.forEach { choice ->
        val mine = pages.plugins.viewModel.sidebar.apps.any { it.tab == choice.tab }
        if (mine) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.myapp_remove)) },
                onClick = {
                    onDismiss()
                    pages.send(Action.Pages(PageAction.RemoveMyApp(MyAppRef.Page(choice.tab))))
                },
            )
        } else {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.myapp_add)) },
                onClick = {
                    onDismiss()
                    pages.send(Action.Pages(PageAction.AddMyApp(MyAppRef.Page(choice.tab), null, null)))
                },
            )
        }
    }
}
