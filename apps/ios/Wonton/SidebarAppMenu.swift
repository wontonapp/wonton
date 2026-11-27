import SwiftUI
import WontonClient

/// “我的应用”一行的长按菜单。整页的应用页顶上不放壳的控件（ADR-0095），页面的操作都在这里：
/// 重新加载、我来操作、详情、新窗口，以及这一插件的每一页加进或移出“我的应用”。
struct SidebarAppMenu: View {
    let app: SidebarApp
    let plugins: PluginsModel
    let pages: PagesModel
    let navigate: (SidebarDestination) -> Void

    var body: some View {
        if case let .page(tab) = app.open {
            Button(TabText.reload) { navigate(.reload(tab)) }
            // 接管只在页面开着时有效（离开页面就结束）：开始以后接着打开这一页。
            if let takeover { TakeoverButton(takeover: takeover) { navigate(.app(app)) } }
            ContentWindowButton(target: .tab(tab: tab))
        }
        if let pluginId = app.pluginId {
            Button(TabText.details) { navigate(.pluginDetails(pluginId)) }
        }
        let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
        ForEach(plugin?.tabChoices ?? []) { choice in
            let added = mine.contains(choice.tab)
            Button {
                if added {
                    pages.send(.pages(action: .removeMyApp(app: .page(tab: choice.tab))))
                } else {
                    pages.send(.pages(action: .addMyApp(app: .page(tab: choice.tab), title: nil, index: nil)))
                }
            } label: {
                Label(
                    added ? TabText.removeFromMyApps : TabText.addToMyApps,
                    systemImage: added ? "minus.circle" : "plus.circle"
                )
            }
        }
    }

    private var plugin: InstalledPlugin? {
        plugins.viewModel.installed?.plugins.first { $0.id == app.pluginId }
    }

    /// 只有声明了 MCP 工具的插件有（ADR-0024）；Bot 网页没有。
    private var takeover: PageTakeover? {
        guard let plugin, plugin.declaresTools else { return nil }
        if case .page(.bot) = app.open { return nil }
        return PageTakeover(pluginId: plugin.id, current: plugins.viewModel.takeover, send: plugins.send)
    }
}
