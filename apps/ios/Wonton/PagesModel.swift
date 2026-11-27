import Observation
import SwiftUI
import WontonClient

@MainActor
@Observable
final class PagesModel {
    private(set) var viewModel: PagesViewModel
    var rejection: Problem?
    let core: ClientCore
    @ObservationIgnored let webViews = TabWebViewStore()
    /// 最近关掉的页面：留几个实例，重开不重载；个数是内存考虑的折中（每个 WebView 一份渲染进程）。
    private(set) var keepAlive: [TabRef] = []

    init(core: ClientCore) {
        self.core = core
        viewModel = core.pages()
    }

    func observe() async {
        for await _ in core.subscribePages().updates() { update(core.pages()) }
    }

    @discardableResult
    func send(_ action: Action) -> Bool {
        do {
            try core.dispatch(action: action)
            update(core.pages())
            rejection = nil
            return true
        } catch {
            rejection = error.problem
            return false
        }
    }

    /// 打开一个页面：已经是栈顶就回到根（再点一次侧边栏的“聊天”是回列表）。
    func open(_ tab: TabRef) {
        send(.pages(action: .openPage(tab: tab)))
    }

    /// 系统入口必须把核心拒绝抛给 Intent，而不是只在 App 内显示弹窗。
    func openForNavigation(_ tab: TabRef) throws {
        try core.dispatch(action: .pages(action: .openPage(tab: tab)))
        update(core.pages())
        rejection = nil
    }
    /// 应用根页露着（栈空且选中的应用项）。
    var appsOpen: Bool { viewModel.tab == .apps && viewModel.stack.isEmpty }

    /// 当前显示的页面：栈顶；栈空时是 Agent 根页。
    var current: TabRef { viewModel.stack.last?.tab ?? .rooms }

    /// 选一个底栏项：两个根页切过去，应用项打开整页。
    func select(_ entry: BarEntry) {
        send(.pages(action: .selectBarItem(entry: entry)))
    }

    /// 栈顶是不是插件网页（聊天、收件箱是原生的；设置是壳的浮层）。
    var showsPluginPage: Bool {
        guard let top = viewModel.stack.last else { return false }
        switch top.tab {
        case .rooms, .inbox: return false
        default: return true
        }
    }

    /// 关掉当前页（系统返回、胶囊的退出）。
    func closeCurrent() {
        send(.pages(action: .closePage))
    }

    /// 主界面把最近用过的页面报给模型，WebView 实例按“栈上的都留 + 最近关掉的留几个”保留。
    func setKeepAlive(_ tabs: [TabRef]) {
        keepAlive = tabs
    }

    private func update(_ view: PagesViewModel) {
        viewModel = view
        let open = Set(view.stack.map { PlacedTab.page(tab: $0.tab) })
        let retained = open.union(keepAlive.map { PlacedTab.page(tab: $0) })
        webViews.retain(retained, open: open)
    }
}

extension OpenPage {
    var title: String {
        switch tab {
        case .rooms: TabText.rooms
        case .inbox: TabText.inbox
        case .plugin, .pluginPath, .bot, .url, .sharedApp, .uiResource, .widget: page.title
        }
    }
}

extension InstalledPlugin {
    var tabChoices: [TabChoice] {
        guard installFailure == nil, webUrl != nil else { return [] }
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
