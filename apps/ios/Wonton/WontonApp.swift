import Foundation
import SwiftUI
import UIKit
import WontonClient

@main
struct WontonApp: App {
    @Environment(\.scenePhase) private var scenePhase
    /// 整个 App 只有一个核心实例，状态都在它里面。
    @State private var launch: Launch
    @State private var tab: AppTab

    init() {
        let launch = Launch.start()
        _launch = State(initialValue: launch)
        _tab = State(initialValue: launch.initialTab)
    }

    var body: some Scene {
        WindowGroup {
            switch launch {
            case let .ready(chat, home, plugins, appearance):
                WontonThemeRoot(appearance: appearance.viewModel) {
                    MainView(chat: chat, home: home, plugins: plugins, appearance: appearance, tab: $tab)
                }
                .task { await appearance.observe() }
                .onChange(of: scenePhase) { old, phase in
                    // 挂起期间服务端可能已经放弃了连接、网络也可能变了，让核心检查并重连。离开后台就发，不等
                    // .active（实测晚约 0.4 秒）：核心要赶在网页恢复后的第一个请求之前开始检查。
                    if old == .background, phase != .background { home.send(.appForegrounded) }
                }
            case let .failed(reason):
                WontonThemeRoot(appearance: defaultAppearance()) {
                    ContentUnavailableView("启动失败", systemImage: "exclamationmark.triangle",
                                           description: Text(reason))
                }
            }
        }
    }
}

private enum AppTab {
    case chat
    case home
    case store
}

/// 主界面的三个页面。主屏和商店里打开插件时，在这里切到聊天，或者全屏打开插件的网页。
private struct MainView: View {
    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    let appearance: AppearanceModel
    @Binding var tab: AppTab
    @State private var page: OpenedPage?
    @State private var unavailable: String?
    @State private var isKeyboardVisible = false

    /// 卡片所在的房间就在眼前时不用提示。
    private var pendingElsewhere: [PendingConfirmation] {
        chat.viewModel.pendingConfirmations.filter { tab != .chat || chat.path.last != .room($0.roomId) }
    }

    var body: some View {
        VStack(spacing: 0) {
            if !pendingElsewhere.isEmpty {
                PendingCardBanner(cards: pendingElsewhere, open: openCardRoom)
            }
            tabs
        }
        .fullScreenCover(item: $page) { page in
            VStack(spacing: 0) {
                if !chat.viewModel.pendingConfirmations.isEmpty {
                    PendingCardBanner(cards: chat.viewModel.pendingConfirmations, open: openCardRoom)
                }
                // 核心重建了本地代理时换上新的凭据。
                PluginPage(
                    title: page.title, url: page.url, proxy: home.viewModel.proxy ?? page.proxy
                ) { self.page = nil }
            }
        }
        .wontonDialog("打不开", message: unavailable, isPresented: isShowingUnavailable)
        // 主屏和商店都用这两份状态，切到别的页面时也要接着更新。提示条在哪个页面都要，聊天也一样。
        .task { await home.observe() }
        .task { await plugins.observe() }
        .task { await chat.observe() }
        .task { await chat.files.observe() }
    }

    private var tabs: some View {
        // 系统的底栏换不了样子，只藏起来。TabView 本身留着：各页的导航、滚动位置和导航栏都由它分开管。
        TabView(selection: $tab) {
            ChatListView(model: chat)
                .toolbar(.hidden, for: .tabBar)
                .tag(AppTab.chat)
            HomeView(
                model: home, plugins: plugins, appearance: appearance, open: open, showStore: { tab = .store },
                devWebURL: Launch.devWebURL
            )
            .toolbar(.hidden, for: .tabBar)
            .tag(AppTab.home)
            StoreView(model: plugins, open: open)
                .toolbar(.hidden, for: .tabBar)
                .tag(AppTab.store)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if showsBottomBar {
                WontonBottomBar(
                    items: [
                        .init(tab: .chat, label: "聊天", icon: .chat, selectedIcon: .chatFill),
                        .init(tab: .home, label: "家", icon: .home, selectedIcon: .homeFill),
                        .init(tab: .store, label: "商店", icon: .store, selectedIcon: .storeFill)
                    ],
                    selection: $tab
                )
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            isKeyboardVisible = true
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
            isKeyboardVisible = false
        }
    }

    private func openCardRoom(_ roomId: String) {
        page = nil
        tab = .chat
        chat.path = [.room(roomId)]
    }

    /// 聊天页和打字时让出底部的位置。
    private var showsBottomBar: Bool {
        !isKeyboardVisible && !(tab == .chat && chat.isRoomOpen)
    }

    private func open(_ entry: PluginEntry) {
        switch entry {
        case let .chat(botUserId):
            tab = .chat
            chat.openDirectChat(with: botUserId)
        case let .web(url, title):
            guard let proxy = home.viewModel.proxy else {
                unavailable = "还没有连上家里的服务器：\(home.viewModel.status.text)"
                return
            }
            page = OpenedPage(title: title, url: url, proxy: proxy)
        }
    }

    private var isShowingUnavailable: Binding<Bool> {
        Binding(
            get: { unavailable != nil },
            set: { isShowing in
                if !isShowing { unavailable = nil }
            }
        )
    }
}

private struct OpenedPage: Identifiable {
    let title: String
    let url: URL
    let proxy: ProxySettings

    var id: URL { url }
}

/// 启动核心。核心失败时如实显示原因，不进入主界面。
@MainActor
private enum Launch {
    case ready(ChatModel, HomeModel, PluginsModel, AppearanceModel)
    case failed(String)

    /// 还没配对时先到“家”的配对页；开发时要自动打开网页的也停在“家”。
    var initialTab: AppTab {
        guard case let .ready(_, home, _, _) = self else { return .chat }
        if case .notConfigured = home.viewModel.status { return .home }
        return Self.devWebURL == nil ? .chat : .home
    }

    /// 开发时 `WONTON_DEV_OPEN_WEB=<插件网页地址>`：本地代理就绪后自动打开（见 HomeView）。
    static var devWebURL: URL? {
        #if DEBUG
        if let text = ProcessInfo.processInfo.environment["WONTON_DEV_OPEN_WEB"] { return URL(string: text) }
        #endif
        return nil
    }

    static func start() -> Launch {
        #if DEBUG
        // WONTON_LOG 可以换过滤规则，例如真机测试时打开本地代理的 debug 日志。
        let filter = ProcessInfo.processInfo.environment["WONTON_LOG"] ?? "wonton_client=info,wonton_tunnel=info,warn"
        initLogging(filter: filter, file: nil)
        #endif
        let core: ClientCore
        do {
            core = try ClientCore(dataDir: dataDirectory().path(percentEncoded: false))
        } catch {
            return .failed("核心启动失败：\(error)")
        }
        // 赶在核心接着传上次没传完的文件之前。
        core.setFileOpener(opener: BookmarkFileOpener())
        #if DEBUG
        // 开发时由 `xcrun simctl launch` / `devicectl` 传入配对码（`dev.sh pair` 的输出），模拟器上不用扫码。
        if let code = ProcessInfo.processInfo.environment["WONTON_DEV_PAIRING_CODE"] {
            do {
                try core.dispatch(action: .pairHome(code: code, deviceName: UIDevice.current.name))
            } catch {
                return .failed("开发时传入的配对码被拒绝：\(error)")
            }
        }
        #endif
        return .ready(
            ChatModel(core: core), HomeModel(core: core), PluginsModel(core: core), AppearanceModel(core: core)
        )
    }

    /// App 私有的数据目录，存设备密钥和连接信息。
    private static func dataDirectory() throws -> URL {
        let base = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        return base.appending(path: "wonton", directoryHint: .isDirectory)
    }
}
