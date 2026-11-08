import SwiftUI
import WontonClient

struct TabPageView: View {
    let tabs: TabsModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let tab: TabRef
    let page: TabPage
    let isOverlay: Bool
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    private let close: (() -> Void)?
    @State private var session: TabWebSession
    @State private var showsDetails = false
    @State private var isVisible = false
    @State private var topHeight: CGFloat = WontonTheme.size.topBar
    @State private var bottomHeight: CGFloat = 0
    /// 全屏胶囊看不看得见。进来、加载完、转屏、手指往下拖时出现，几秒没碰就藏起来。
    @State private var capsuleShown = true
    @State private var capsuleShowings = 0
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonTabBar) private var bottomBar
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var textScale: CGFloat = 1

    init(
        tabs: TabsModel, home: HomeModel, plugins: PluginsModel, chat: ChatModel, appearance: AppearanceModel,
        tab: TabRef, page: TabPage, isOverlay: Bool,
        draft: @escaping (String) -> Void, open: @escaping (PluginEntry) -> Void,
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String) -> Void,
        close: (() -> Void)? = nil
    ) {
        self.tabs = tabs
        self.home = home
        self.plugins = plugins
        self.chat = chat
        self.appearance = appearance
        self.tab = tab
        self.page = page
        self.isOverlay = isOverlay
        self.draft = draft
        self.open = open
        self.pendingCards = pendingCards
        self.openCardRoom = openCardRoom
        self.close = close
        _session = State(initialValue: tabs.webViews.session(for: tab, core: tabs.core))
    }

    var body: some View {
        GeometryReader { geometry in
            let safe = geometry.safeAreaInsets
            let frame = webFrame(safe: safe)
            let inputs = inputs(safe: safe)
            ZStack {
                if let webView = session.webView {
                    TabWebView(webView: webView, obscured: obscured(safe: safe))
                    .id(ObjectIdentifier(webView))
                    .padding(frame)
                    .ignoresSafeArea(.container)
                }
                TabPageStateView(
                    page: page, session: session, connected: connected,
                    details: { showsDetails = true },
                    remove: {
                        if let close { close() } else { tabs.send(isOverlay ? .closeTabOverlay : .unpinTab(tab: tab)) }
                    }
                )
                .padding(insetFrame(safe: safe))
                .ignoresSafeArea(.container)
                chrome(safe: safe)
            }
            .background { Color(colors.canvas).ignoresSafeArea(.container) }
            .background {
                FullscreenExitGesture(
                    enabled: page.fullscreen && isVisible && !showsDetails, webView: session.webView,
                    exit: { tabs.send(.exitFullscreen(tab: tab)) }
                )
            }
            .background {
                FullscreenCapsuleReveal(enabled: page.fullscreen && isVisible, webView: session.webView) { show in
                    if show { showCapsule() } else { capsuleShown = false }
                }
            }
            .task(id: inputs) { updateSession(inputs) }
            .onChange(of: geometry.size) { showCapsule() }
        }
        // 键盘交给 WebKit 自己让（和 Safari 一样：可视区域缩小，焦点输入框滚进来）。SwiftUI 默认再把网页框缩到键盘上面，
        // 两边各让一次：网页只剩一条，noVNC 这类按窗口缩放的页面跟着缩，缩放动画里画面被拉成竖条（iOS 26.5 实测）。
        .ignoresSafeArea(.keyboard)
        .onChange(of: page.fullscreen, initial: true) { showCapsule() }
        .onChange(of: session.ready) { showCapsule() }
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

    /// 带栏时没适配的页面限在标题栏和底栏之间，不铺到底栏后面：页面自己的底部按钮栏会和底栏叠在一起。
    /// 全屏时从状态栏下面开始，底边铺到屏幕底，iOS 26 起由 WebKit 的 `obscuredContentInsets` 让出 Home 条那块
    /// （固定定位的元素和滚动的终点停在它上面），更早的系统停在 Home 条上面。顶边不用这个接口：iOS 26.5 上
    /// 切换全屏时它会把顶边弄丢，页面钻到状态栏后面。
    private func webFrame(safe: EdgeInsets) -> EdgeInsets {
        if edgeToEdge { return EdgeInsets() }
        let frame = insetFrame(safe: safe)
        if #available(iOS 26, *), page.fullscreen {
            return EdgeInsets(top: frame.top, leading: frame.leading, bottom: 0, trailing: frame.trailing)
        }
        return frame
    }

    private func obscured(safe: EdgeInsets) -> UIEdgeInsets {
        guard !edgeToEdge else { return .zero }
        let frame = webFrame(safe: safe)
        return UIEdgeInsets(top: 0, left: 0, bottom: insetFrame(safe: safe).bottom - frame.bottom, right: 0)
    }

    private func insetFrame(safe: EdgeInsets) -> EdgeInsets {
        EdgeInsets(top: safe.top + (page.fullscreen ? 0 : topHeight), leading: safe.leading,
                   bottom: safe.bottom + contentBottom, trailing: safe.trailing)
    }

    private func showCapsule() {
        capsuleShown = true
        capsuleShowings += 1
    }

    @ViewBuilder private func chrome(safe: EdgeInsets) -> some View {
        if page.fullscreen {
            VStack(spacing: 0) {
                TabPageCapsule(
                    page: page, tab: tab, tabs: tabs, session: session, takeover: takeover,
                    details: { showsDetails = true }, shown: capsuleShown || voiceOver, touched: showCapsule
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

    private var edgeToEdge: Bool { page.layout == .edgeToEdge && (!isOverlay || page.fullscreen) }
    private var contentBottom: CGFloat { page.fullscreen || isOverlay || bottomBar == nil ? 0 : bottomHeight }
    private var connected: Bool { home.viewModel.status == .connected }
    private var pluginId: String? {
        if case let .plugin(id, _) = tab { return id }
        return nil
    }

    private var takeover: PageTakeover? {
        guard let pluginId, let plugin = plugins.viewModel.installed?.plugins.first(where: { $0.id == pluginId }),
              plugin.declaresTools else { return nil }
        return PageTakeover(pluginId: pluginId, current: plugins.viewModel.takeover, send: plugins.send)
    }

    private func inputs(safe: EdgeInsets) -> TabWebInputs {
        let bottom = contentBottom
        return TabWebInputs(
            page: page, proxy: home.viewModel.proxy, connected: connected,
            appearance: appearance.viewModel,
            environment: TabEnvironment(
                locale: AppLanguage.current,
                timeZone: TimeZone.current.identifier, platform: .ios, dark: colorScheme == .dark,
                insetTop: edgeToEdge ? Double(safe.top + (page.fullscreen ? 0 : topHeight)) : 0,
                insetBottom: edgeToEdge ? Double(bottom + safe.bottom) : 0,
                safeArea: TabSafeArea(
                    top: edgeToEdge ? Double(safe.top) : 0, right: edgeToEdge ? Double(safe.trailing) : 0,
                    bottom: edgeToEdge ? Double(safe.bottom) : 0, left: edgeToEdge ? Double(safe.leading) : 0
                )
            ), textScale: textScale
        )
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
                        session?.problem = Problem(title: String(localized: .tabOpenLinkFailed))
                    }
                }
            case let .draftMessage(text): draft(text)
            }
        }
    }
}
