import SwiftUI
import WontonClient

struct TabPageView: View {
    let tabs: TabsModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let tab: TabRef
    let placed: PlacedTab
    let store: TabWebViewStore
    let page: TabPage
    let isOverlay: Bool
    /// 面板只留当前页和左右各一页（ADR-0052）：不在窗口里的释放 WebView，回来了重新建。
    let keepAlive: Bool
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    private let close: (() -> Void)?
    /// 面板里的那一份（ADR-0052）：整页大小、右上角胶囊、退出回聊天页。
    private let panel: TabPanelMode?
    private let remove: (() -> Void)?
    @State private var session: TabWebSession
    @State private var showsDetails = false
    @State private var isVisible = false
    @State private var topHeight: CGFloat = WontonTheme.size.topBar
    @State private var bottomHeight: CGFloat = 0
    /// 全屏胶囊看不看得见。进来、切回 App、加载完、转屏、手指往下拖时出现，几秒没碰就藏起来。
    @State private var capsuleShown = true
    @State private var capsuleShowings = 0
    @State private var seenReady = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonTabBar) private var bottomBar
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var textScale: CGFloat = 1

    init(
        tabs: TabsModel, home: HomeModel, plugins: PluginsModel, chat: ChatModel, appearance: AppearanceModel,
        tab: TabRef, placed: PlacedTab, store: TabWebViewStore, page: TabPage, isOverlay: Bool,
        keepAlive: Bool = true, draft: @escaping (String) -> Void, open: @escaping (PluginEntry) -> Void,
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String) -> Void,
        close: (() -> Void)? = nil, panel: TabPanelMode? = nil, remove: (() -> Void)? = nil
    ) {
        self.tabs = tabs
        self.home = home
        self.plugins = plugins
        self.chat = chat
        self.appearance = appearance
        self.tab = tab
        self.page = page
        self.isOverlay = isOverlay
        self.keepAlive = keepAlive
        self.draft = draft
        self.open = open
        self.pendingCards = pendingCards
        self.openCardRoom = openCardRoom
        self.close = close
        self.panel = panel
        self.remove = remove
        self.placed = placed
        self.store = store
        _session = State(initialValue: store.session(for: placed, core: tabs.core))
    }

    var body: some View {
        content
        // 键盘交给 WebKit 自己让（和 Safari 一样：可视区域缩小，焦点输入框滚进来）。SwiftUI 默认再把网页框缩到键盘上面，
        // 两边各让一次：网页只剩一条，noVNC 这类按窗口缩放的页面跟着缩，缩放动画里画面被拉成竖条（iOS 26.5 实测）。
        .ignoresSafeArea(.keyboard)
        .environment(\.wontonBarStatus, pageStatus)
        .onChange(of: page.fullscreen, initial: true) { showCapsule() }
        .onChange(of: panel?.expanded) { showCapsule() }
        .onChange(of: session.ready) { showCapsule(); notifySeen() }
        .onChange(of: panel?.visible) { _, visible in
            if visible == true { seenReady = false; notifySeen() }
        }
        .onAppear { notifySeen() }
        // 切回 App 也算“进来”：全屏页和面板整页在后台时胶囊早藏了，回来不叫出来就看不到出口。
        .onChange(of: scenePhase) { _, phase in if phase == .active { showCapsule() } }
        .task(id: capsuleShowings) {
            try? await Task.sleep(for: .seconds(3))
            if !Task.isCancelled { capsuleShown = false }
        }
        .onChange(of: bottomBar == nil) { _, absent in if absent { bottomHeight = 0 } }
        .onAppear { isVisible = true }
        .onDisappear { isVisible = false; takeover?.end() }
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
            let frame = webFrame(safe: safe)
            let inputs = inputs(safe: safe)
            panelStack(safe: safe, frame: frame, inputs: inputs)
                .task(id: inputs) { updateSession(inputs) }
                .onChange(of: keepAlive) { _, keep in keepSessionAlive(inputs, when: keep) }
                .task(id: store.generation) { keepSessionAlive(inputs, when: keepAlive) }
                .onChange(of: geometry.size) { showCapsule() }
        }
    }

    @ViewBuilder
    private func panelStack(safe: EdgeInsets, frame: EdgeInsets, inputs: TabWebInputs) -> some View {
        ZStack {
                if let webView = session.webView, sourceAvailable {
                    TabWebView(webView: webView, obscured: obscured(safe: safe))
                    .id(ObjectIdentifier(webView))
                    .padding(frame)
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
                            tabs.send(isOverlay ? .closeTabOverlay : .unpinTab(tab: tab))
                        }
                    }
                )
                .padding(insetFrame(safe: safe))
                .ignoresSafeArea(.container)
                chrome(safe: safe)
            }
            .background { Color(colors.canvas).ignoresSafeArea(.container) }
            .background {
                FullscreenExitGesture(
                    enabled: panel == nil && page.fullscreen && isVisible && !showsDetails, webView: session.webView,
                    exit: { tabs.send(.exitFullscreen(tab: tab)) }
                )
            }
            .background {
                FullscreenCapsuleReveal(enabled: capsuleAllowed && isVisible, webView: session.webView) { show in
                    if show { showCapsule() } else { capsuleShown = false }
                }
            }
            .background {
                if let panel {
                    PanelPagePull(
                        enabled: panel.pagePull && isVisible, webView: session.webView,
                        changed: panel.pageChanged, ended: panel.pageEnded
                    )
                }
            }
    }
}

extension TabPageView {
    /// 带栏时没适配的页面限在标题栏和底栏之间，不铺到底栏后面：页面自己的底部按钮栏会和底栏叠在一起。
    /// 全屏时从状态栏下面开始，底边铺到屏幕底，iOS 26 起由 WebKit 的 `obscuredContentInsets` 让出 Home 条那块
    /// （固定定位的元素和滚动的终点停在它上面），更早的系统停在 Home 条上面。顶边不用这个接口：iOS 26.5 上
    /// 切换全屏时它会把顶边弄丢，页面钻到状态栏后面。
    private func webFrame(safe: EdgeInsets) -> EdgeInsets {
        if let panel {
            // 面板里的网页一直是整页大小，级之间只挪位置、不缩放（ADR-0052）：顶边的状态栏和底边的
            // 遮挡经 `obscuredContentInsets` 让出，不改网页框。
            return EdgeInsets(top: 0, leading: safe.leading, bottom: 0, trailing: safe.trailing)
        }
        if edgeToEdge { return EdgeInsets() }
        let frame = insetFrame(safe: safe)
        if #available(iOS 26, *), page.fullscreen {
            return EdgeInsets(top: frame.top, leading: frame.leading, bottom: 0, trailing: frame.trailing)
        }
        return frame
    }

    private func obscured(safe: EdgeInsets) -> UIEdgeInsets {
        if let panel {
            return UIEdgeInsets(top: panel.insetTop, left: 0, bottom: panel.insetBottom, right: 0)
        }
        guard !edgeToEdge else { return .zero }
        let frame = webFrame(safe: safe)
        return UIEdgeInsets(top: 0, left: 0, bottom: insetFrame(safe: safe).bottom - frame.bottom, right: 0)
    }

    private func insetFrame(safe: EdgeInsets) -> EdgeInsets {
        if let panel {
            return EdgeInsets(
                top: panel.insetTop, leading: safe.leading, bottom: panel.insetBottom, trailing: safe.trailing
            )
        }
        return EdgeInsets(top: safe.top + (page.fullscreen ? 0 : topHeight), leading: safe.leading,
                          bottom: safe.bottom + contentBottom, trailing: safe.trailing)
    }

    /// 面板拉开、翻到这一页只发一次“被看到了”，重新翻回来再发。
    private func notifySeen() {
        guard panel?.visible == true, session.ready, !seenReady else { return }
        seenReady = true
        session.seen()
    }

    private func showCapsule() {
        capsuleShown = true
        capsuleShowings += 1
    }

    @ViewBuilder private func chrome(safe: EdgeInsets) -> some View {
        if fullscreen {
            VStack(spacing: 0) {
                TabPageCapsule(
                    page: page, tab: tab, placed: placed, tabs: tabs, session: session, takeover: takeover,
                    details: { showsDetails = true },
                    shown: capsuleAllowed && (capsuleShown || voiceOver), touched: showCapsule, panel: panel
                )
                .padding(.top, safe.top)
                .padding(.leading, safe.leading + WontonTheme.spacing.s8)
                .padding(.trailing, safe.trailing + WontonTheme.spacing.s8)
                if !pendingCards.isEmpty {
                    PendingCardBanner(cards: pendingCards, open: openCardRoom)
                }
                if let takeover { TakeoverNotice(takeover: takeover) }
                Spacer(minLength: 0)
            }
            .ignoresSafeArea(.container)
        } else {
            VStack(spacing: 0) {
                TabPageTitleBar(
                    page: page, session: session, takeover: takeover, isOverlay: isOverlay, material: edgeToEdge,
                    enterFullscreen: { tabs.send(.enterFullscreen(tab: tab)) }, close: closeOverlay
                )
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { topHeight = $0 }
                if let takeover { TakeoverNotice(takeover: takeover) }
                Spacer(minLength: 0)
                if !isOverlay, let bottomBar {
                    bottomBar
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { bottomHeight = $0 }
                }
            }
        }
    }

    private func closeOverlay() {
        if let close { close() } else { tabs.send(.closeTabOverlay) }
    }

    /// 面板里的页面按整页显示：状态栏照常显示，右上角是胶囊（ADR-0052）。
    private var fullscreen: Bool { page.fullscreen || panel != nil }
    private var capsuleAllowed: Bool { panel?.expanded ?? page.fullscreen }
    private var edgeToEdge: Bool { panel != nil || page.layout == .edgeToEdge && (!isOverlay || page.fullscreen) }
    private var contentBottom: CGFloat { fullscreen || isOverlay || bottomBar == nil ? 0 : bottomHeight }
    private var connected: Bool {
        if case .sharedApp = tab { return sourceProxy != nil }
        return home.viewModel.status == .connected
    }
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

    private func inputs(safe: EdgeInsets) -> TabWebInputs {
        let bottom = contentBottom
        // 面板里的页面按铺满的布局给核心：inset 布局的变量在核心那边会被归零，页面对不上安全区。
        var inputPage = page
        if panel != nil { inputPage.layout = .edgeToEdge }
        return TabWebInputs(
            page: inputPage, proxy: sourceProxy, connected: connected,
            appearance: appearance.viewModel,
            environment: TabEnvironment(
                locale: AppLanguage.current,
                timeZone: TimeZone.current.identifier, platform: .ios, dark: colorScheme == .dark,
                insetTop: edgeToEdge ? Double(panel?.insetTop ?? safe.top + (fullscreen ? 0 : topHeight)) : 0,
                insetBottom: edgeToEdge ? Double(panel?.insetBottom ?? bottom + safe.bottom) : 0,
                safeArea: TabSafeArea(
                    top: edgeToEdge ? Double(safe.top) : 0, right: edgeToEdge ? Double(safe.trailing) : 0,
                    bottom: edgeToEdge ? Double(safe.bottom) : 0, left: edgeToEdge ? Double(safe.leading) : 0
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
        session = store.session(for: placed, core: tabs.core)
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
