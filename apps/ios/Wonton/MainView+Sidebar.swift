import SwiftUI
import WontonClient

extension MainView {
    /// 只在聊天的根页面响应左边缘右滑：进了页面或盖了别的层，左边缘是返回手势。
    var allowsEdgeSwipe: Bool {
        !pages.showsPluginPage && pagePath.isEmpty && chat.path.isEmpty && appsPath.isEmpty
    }

    /// 侧边栏的入口：先收起侧边栏（SidebarContainer 做），再按目的地跳。
    func navigate(_ destination: SidebarDestination) {
        showsSidebar = false
        switch destination {
        case .chat:
            pages.open(.chat)
            chat.path = []
        case .store:
            openApps(above: [.store])
        case .allApps:
            openApps()
        case .settings:
            settings.path = []
            showsSettings = true
        case .newChat:
            pages.open(.chat)
            chat.path = [.startChat]
        case let .reload(tab):
            // 核心只重新加载栈里的页面：先打开，打开被拒了就只报那一条。
            if pages.send(.pages(action: .openPage(tab: tab))) {
                pages.send(.pages(action: .reloadTab(page: .page(tab: tab))))
            }
        case let .pluginDetails(id):
            openApps()
            appsPath.append(PluginRoute.installed(id: id))
        case let .app(app):
            switch app.open {
            case .botChat(let botUserId):
                open(.chat(botUserId: botUserId))
            case .page(let tab):
                pages.open(tab)
            case .externalBrowser(let url):
                // 外部网页插件：系统的 App 内浏览器，顶上显示网址，没有和 App 通信的桥（ADR-0080）。
                guard let url = URL(string: url) else { return }
                navigation.safari = SafariTarget(url: url)
            }
        }
    }
}
