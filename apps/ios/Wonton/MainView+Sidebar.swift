import SwiftUI
import WontonClient

extension MainView {
    /// 只在聊天的根页面响应左边缘右滑：进了房间，左边缘是系统的返回手势。
    /// 聊天页内的下拉面板（ADR-0052）展开时也不响应：手势归面板翻页，等面板那边把“展开了”交到根视图。
    var allowsEdgeSwipe: Bool {
        tabs.viewModel.selected == .chat && chat.path.isEmpty && tabs.viewModel.overlay == nil
    }

    func navigate(_ destination: SidebarDestination) {
        showsSidebar = false
        switch destination {
        case .chat:
            tabs.select(.chat)
            chat.path = []
        case .store:
            tabs.select(.apps)
            storeRequest += 1
        case .allApps:
            tabs.select(.apps)
        case .settings:
            if tabs.isPinned(.settings) {
                tabs.select(.settings)
            } else {
                settings.path = []
                showsSettings = true
            }
        case .newChat:
            tabs.select(.chat)
            chat.path = [.startChat]
        case let .app(app):
            switch app.open {
            case .botChat(let botUserId):
                open(.chat(botUserId: botUserId))
            case .page(let tab):
                tabs.select(tab)
            }
        }
    }
}
