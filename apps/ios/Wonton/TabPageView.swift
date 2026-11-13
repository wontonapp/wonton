import SwiftUI
import WontonClient

/// 整页的插件网页：铺满、右上角胶囊；聊天页下拉面板里的那份用 `panel` 传布局。
struct TabPageView: View {
    let pages: PagesModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let tab: TabRef
    let placed: PlacedTab
    let store: TabWebViewStore
    let page: TabPage
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    private let close: (() -> Void)?
    /// 面板里的那一份（ADR-0052）：整页大小、右上角胶囊、退出回聊天页。
    private let panel: TabPanelMode?
    private let remove: (() -> Void)?
    /// 面板只留当前页和左右各一页（ADR-0052）：不在窗口里的释放 WebView，回来了重新建。
    private let keepAlive: Bool
    @State private var session: TabWebSession
    @State private var showsDetails = false
    @State private var isVisible = false
    @State private var seenReady = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var textScale: CGFloat = 1

    init(
        pages: PagesModel, home: HomeModel, plugins: PluginsModel, chat: ChatModel, appearance: AppearanceModel,
        tab: TabRef, placed: PlacedTab, store: TabWebViewStore, page: TabPage,
        draft: @escaping (String) -> Void, open: @escaping (PluginEntry) -> Void,
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String) -> Void,
        close: (() -> Void)? = nil, panel: TabPanelMode? = nil, remove: (() -> Void)? = nil,
        keepAlive: Bool = true
    ) {
        self.pages = pages
        self.home = home
        self.plugins = plugins
        self.chat = chat
        self.appearance = appearance
        self.tab = tab
        self.page = page
        self.draft = draft
        self.open = open
        self.pendingCards = pendingCards
        self.openCardRoom = openCardRoom
        self.close = close
        self.panel = panel
        self.remove = remove
        self.keepAlive = keepAlive
        self.placed = placed
        self.store = store
        _session = State(initialValue: store.session(for: placed, core: pages.core))
    }

    var body: some View {
        content
        // 键盘交给 WebKit 自己让（和 Safari 一样：可视区域缩小，焦点输入框滚进来）。SwiftUI 默认再把网页框缩到键盘上面，
        // 两边各让一次：网页只剩一条，noVNC 这类按窗口缩放的页面跟着缩，缩放动画里画面被拉成竖条（iOS 26.5 实测）。
        .ignoresSafeArea(.keyboard)
        .environment(\.wontonBarStatus, pageStatus)
        .onChange(of: session.ready) { notifySeen() }
        .onChange(of: panel?.visible) { _, visible in
            if visible == true { seenReady = false; notifySeen() }
        }
        .onAppear { notifySeen(); isVisible = true }
        .onDisappear {
            isVisible = false
            if panel == nil { takeover?.end() }
        }
        .sheet(isPresented: $showsDetails) {
            if let pluginId {
                NavigationStack {
                    PluginDetailView(model: plugins, chat: chat, route: .installed(id: pluginId)) { entry in
                        showsDetails = false
                        open(entry)
                    }
                }
            }
        }
    }

    /// 拆开写：整段放在 `body` 里时，Xcode 27 的编译器会报“类型检查超时”（CI 的模拟器构建因此失败）。
    private var content: some View {
        GeometryReader { geometry in
            let safe = geometry.safeAreaInsets
            let inputs = inputs(safe: safe)
            ZStack {
                if let webView = session.webView, sourceAvailable {
                    TabWebView(webView: webView, obscured: obscured())
                        .id(ObjectIdentifier(webView))
                        .ignoresSafeArea(.container)
                }
                TabPageStateView(
                    page: page, session: session, connected: connected,
                    shareProblem: plugins.viewModel.sharedProblem(for: tab), isPanel: panel != nil,
                    details: { showsDetails = true },
                    remove: {
                        if let remove {
                            remove()
                        } else if let close {
                            close()
                        } else {
                            pages.send(.closePage)
                        }
                    }
                )
                .ignoresSafeArea(.container)
                chrome(safe: safe)
            }
            .background { Color(colors.canvas).ignoresSafeArea(.container) }
            .background {
                if let panel {
                    PanelPagePull(
                        enabled: panel.pagePull && isVisible, webView: session.webView, edge: panel.pageEdge,
                        zone: PagePullZone(obscured: obscured()),
                        changed: panel.pageChanged, ended: panel.pageEnded, cancelled: panel.pageCancelled
                    )
                }
            }
            .task(id: inputs) { updateSession(inputs) }
            .onChange(of: keepAlive) { _, keep in keepSessionAlive(inputs, when: keep) }
            .task(id: store.generation) { keepSessionAlive(inputs, when: keepAlive) }
        }
    }

    @ViewBuilder private func chrome(safe: EdgeInsets) -> some View {
        VStack(spacing: 0) {
            TabPageCapsule(
                page: page, tab: tab, placed: placed, pages: pages, session: session, takeover: takeover,
                details: { showsDetails = true },
                shown: capsuleAllowed, panel: panel,
                close: close ?? { pages.send(.closePage) }
            )
            // 面板里按网页框的顶边放：房间的面板第 1 级从屏幕中间开始，那里没有状态栏要让。
            .padding(.top, panel?.insetTop ?? safe.top)
            .padding(.leading, safe.leading + WontonTheme.spacing.s8)
            .padding(.trailing, safe.trailing + WontonTheme.spacing.s8)
            if panel == nil, !pendingCards.isEmpty {
                PendingCardBanner(cards: pendingCards, open: openCardRoom)
            }
            if let takeover { TakeoverNotice(takeover: takeover) }
            Spacer(minLength: 0)
        }
        .ignoresSafeArea(.container)
    }

    /// 面板拉开、翻到这一页只发一次“被看到了”，重新翻回来再发。
    private func notifySeen() {
        guard panel?.visible == true, session.ready, !seenReady else { return }
        seenReady = true
        session.seen()
    }

    /// 面板拉开着就有胶囊，两级都是（ADR-0052）：第 1 级只靠点露出的那条聊天页收回时，页面出错或空白就找不到出口。
    private var capsuleAllowed: Bool { panel?.open ?? true }

    private var connected: Bool {
        if case .sharedApp = tab { return sourceProxy != nil }
        return home.viewModel.status == .connected
    }

    /// 分享页走分享来源自己的代理；普通页面走家里的隧道。
    private var sourceProxy: ProxySettings? {
        if case .sharedApp = tab {
            guard let source = plugins.viewModel.sharedSource(for: tab),
                  case let .ready(proxy) = source.state else { return nil }
            return proxy
        }
        return home.viewModel.proxy
    }

    private var sourceAvailable: Bool {
        if case .sharedApp = tab { return connected && plugins.viewModel.sharedProblem(for: tab) == nil }
        return true
    }

    /// 分享页的连接条写清来源，不说成家里服务器的问题。
    private var pageStatus: WontonBarStatus {
        guard case .sharedApp = tab else { return status }
        let title = plugins.viewModel.sharedSource(for: tab).map {
            $0.ownerDisplayName.flatMap { $0.isEmpty ? nil : $0 } ?? $0.ownerUserId
        }
        return WontonBarStatus(subtitle: title)
    }
    private var pluginId: String? {
        switch tab {
        case let .plugin(id, _), let .pluginPath(id, _): id
        default: nil
        }
    }

    private var takeover: PageTakeover? {
        guard let pluginId, let plugin = plugins.viewModel.installed?.plugins.first(where: { $0.id == pluginId }),
              plugin.declaresTools else { return nil }
        return PageTakeover(pluginId: pluginId, current: plugins.viewModel.takeover, send: plugins.send)
    }

    /// 整页铺满整个屏幕、不替页面让安全区（ADR-0095）：刘海和 Home 条由页面自己按 env(safe-area-inset-*) 或桥给的
    /// 安全区让。面板里的页面照面板的布局让出上下两条（ADR-0052）。
    private func obscured() -> UIEdgeInsets {
        guard let panel else { return .zero }
        return UIEdgeInsets(top: panel.insetTop, left: 0, bottom: panel.insetBottom, right: 0)
    }

    private func inputs(safe: EdgeInsets) -> TabWebInputs {
        let bottom = panel?.insetBottom ?? 0
        let top = panel?.insetTop ?? safe.top
        return TabWebInputs(
            page: page, proxy: sourceProxy, connected: connected,
            appearance: appearance.viewModel,
            environment: TabEnvironment(
                locale: AppLanguage.current,
                timeZone: TimeZone.current.identifier, platform: .ios, dark: colorScheme == .dark,
                insetTop: Double(top), insetBottom: Double(bottom + (panel == nil ? safe.bottom : 0)),
                safeArea: TabSafeArea(
                    top: Double(safe.top), right: Double(safe.trailing),
                    bottom: Double(safe.bottom), left: Double(safe.leading)
                )
            ), textScale: textScale
        )
    }

    /// 会话被释放过（`TabWebViewStore.retain` 放掉的）：重新要一份并加载。
    /// 面板只留当前页和左右各一页（ADR-0052）。
    private func keepSessionAlive(_ inputs: TabWebInputs, when keep: Bool) {
        if keep, session.webView == nil { revive(inputs) }
    }

    private func revive(_ inputs: TabWebInputs) {
        session = store.session(for: placed, core: pages.core)
        updateSession(inputs)
    }

    private func updateSession(_ inputs: TabWebInputs) {
        let openURL = openURL
        let draft = draft
        session.update(inputs) { [weak session] action in
            switch action {
            case let .openLink(value):
                guard let url = URL(string: value) else { return }
                openURL(url) { accepted in
                    if !accepted {
                        session?.problem = Problem(title: String(localized: .Tab0.tabOpenLinkFailed))
                    }
                }
            case let .draftMessage(text): draft(text)
            }
        }
    }
}
