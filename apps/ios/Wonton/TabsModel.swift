import Observation
import SwiftUI
import WontonClient

@MainActor
@Observable
final class TabsModel {
    private(set) var viewModel: TabsViewModel
    var rejection: Problem?
    let core: ClientCore
    @ObservationIgnored let webViews = TabWebViewStore()

    init(core: ClientCore) {
        self.core = core
        viewModel = core.tabs()
    }

    func observe() async {
        for await _ in core.subscribeTabs().updates() { update(core.tabs()) }
    }

    @discardableResult
    func send(_ action: Action) -> Bool {
        do {
            try core.dispatch(action: action)
            update(core.tabs())
            rejection = nil
            return true
        } catch {
            rejection = error.problem
            return false
        }
    }

    func select(_ tab: TabRef) {
        if viewModel.selected == tab, viewModel.overlay == nil { webViews.reselect(.bar(tab: tab)) }
        send(.selectTab(tab: tab))
    }

    /// 系统入口必须把核心拒绝抛给 Intent，而不是只在 App 内显示弹窗。
    func selectForNavigation(_ tab: TabRef) throws {
        try core.dispatch(action: .selectTab(tab: tab))
        update(core.tabs())
        rejection = nil
    }

    func isPinned(_ tab: TabRef) -> Bool { viewModel.items.contains { $0.tab == tab } }

    private func update(_ view: TabsViewModel) {
        viewModel = view
        var retained = Set(view.items.map { PlacedTab.bar(tab: $0.tab) })
        if let overlay = view.overlay { retained.insert(.bar(tab: overlay.tab)) }
        webViews.retain(retained)
    }
}

extension TabItem {
    var title: String {
        switch tab {
        case .chat: TabText.chat
        case .apps: TabText.apps
        case .settings: TabText.settings
        case .plugin, .pluginPath, .bot, .url, .sharedApp: page?.title ?? TabText.apps
        }
    }
}

extension InstalledPlugin {
    var tabChoices: [TabChoice] {
        guard webUrl != nil else { return [] }
        if tabs.isEmpty {
            return [TabChoice(tab: .plugin(pluginId: id, page: nil), title: title)]
        }
        return tabs.map { TabChoice(tab: .plugin(pluginId: id, page: $0.id), title: $0.title) }
    }
}

struct TabChoice: Identifiable {
    let tab: TabRef
    let title: String
    var id: TabRef { tab }
}
