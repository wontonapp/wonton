import SwiftUI
import WontonClient

struct TabPageView: View {
    let tabs: TabsModel
    let home: HomeModel
    let plugins: PluginsModel
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
    @State private var edges = TabPageEdges()
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonTabBar) private var bottomBar
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var textScale: CGFloat = 1

    init(
        tabs: TabsModel, home: HomeModel, plugins: PluginsModel, appearance: AppearanceModel,
        tab: TabRef, page: TabPage, isOverlay: Bool,
        draft: @escaping (String) -> Void, open: @escaping (PluginEntry) -> Void,
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String) -> Void,
        close: (() -> Void)? = nil
    ) {
        self.tabs = tabs
        self.home = home
        self.plugins = plugins
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
                    TabWebView(webView: webView)
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
            .background {
                TabPageBackdrop(edges: edges, gaps: backdropGaps(frame), fallback: backdropFallback)
                    .ignoresSafeArea(.container)
            }
            .background {
                FullscreenExitGesture(
                    enabled: page.fullscreen && isVisible && !showsDetails, webView: session.webView,
                    exit: { tabs.send(.exitFullscreen(tab: tab)) }
                )
            }
            .task(id: inputs) { updateSession(inputs) }
            .task(id: EdgeSampling(edges: backdropGaps(frame).sampledEdges, visible: isVisible,
                                   webView: session.webView.map(ObjectIdentifier.init))) {
                await sampleEdges(backdropGaps(frame).sampledEdges)
            }
        }
        .onChange(of: bottomBar == nil) { _, absent in if absent { bottomHeight = 0 } }
        .onAppear { isVisible = true }
        .onDisappear { isVisible = false; takeover?.end() }
        .sheet(isPresented: $showsDetails) {
            if let pluginId {
                NavigationStack {
                    PluginDetailView(model: plugins, route: .installed(id: pluginId)) { entry in
                        showsDetails = false
                        open(entry)
                    }
                }
            }
        }
    }

    /// 全屏时是网页框到屏幕四边的距离（顶栏、胶囊和系统安全区都算在内）；带栏时是网页框在安全区里的留白。
    private func webFrame(safe: EdgeInsets) -> EdgeInsets {
        edgeToEdge ? EdgeInsets() : insetFrame(safe: safe)
    }

    private func insetFrame(safe: EdgeInsets) -> EdgeInsets {
        if page.fullscreen {
            return EdgeInsets(top: max(safe.top, topHeight), leading: safe.leading,
                              bottom: safe.bottom, trailing: safe.trailing)
        }
        return EdgeInsets(top: safe.top + topHeight, leading: safe.leading,
                          bottom: safe.bottom + contentBottom, trailing: safe.trailing)
    }

    /// 带栏时顶部是 App 自己的栏，不画；底部浮着的底栏下面、没有底栏时 Home 条那一条，都画网页的底边。
    private func backdropGaps(_ frame: EdgeInsets) -> EdgeInsets {
        if page.fullscreen { return frame }
        return EdgeInsets(top: 0, leading: 0, bottom: frame.bottom, trailing: 0)
    }

    private var backdropFallback: Color {
        if page.fullscreen, session.ready, let base = edges.base { return Color(uiColor: base) }
        return Color(colors.canvas)
    }

    /// 页面里的固定顶栏、底栏随时会变（换页、换主题），没有通知，所以看得见时每秒取一次。
    private func sampleEdges(_ sides: Set<Edge>) async {
        guard isVisible, !sides.isEmpty else { return }
        while !Task.isCancelled {
            if let webView = session.webView { edges.sample(webView, edges: sides) }
            try? await Task.sleep(for: .seconds(1))
        }
    }

    @ViewBuilder private func chrome(safe: EdgeInsets) -> some View {
        if page.fullscreen {
            let band = capsuleFitsBesideCamera(safe: safe)
            VStack(spacing: 0) {
                TabFullscreenBar(page: page, tab: tab, tabs: tabs, details: { showsDetails = true })
                    .padding(.top, band ? (safe.top - WontonTheme.size.smallButton) / 2
                             : safe.top + WontonTheme.spacing.s8)
                    .padding(.leading, safe.leading + WontonTheme.spacing.s8)
                    .padding(.trailing, band ? WontonTheme.spacing.s24 : safe.trailing + WontonTheme.spacing.s8)
                    .padding(.bottom, WontonTheme.spacing.s8)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { topHeight = $0 }
                if !pendingCards.isEmpty {
                    PendingCardBanner(cards: pendingCards, open: openCardRoom)
                }
                if let takeover { TakeoverNotice(takeover: takeover) }
                Spacer(minLength: 0)
            }
            .ignoresSafeArea(.container)
        } else {
            VStack(spacing: 0) {
                topBar
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

    /// 灵动岛的机型隐藏状态栏后，顶部安全区（59pt 以上）在岛两边空着，胶囊放进右边那块，不压页面。
    /// 刘海屏（44–50pt）的刘海宽度各代不同、系统不给，放进去可能压到刘海；没有刘海的机型和横屏，顶部安全区是 0。
    /// 这些情况放在安全区下面，页面让出这一条。短状态要占左边，也放到下面。
    private func capsuleFitsBesideCamera(safe: EdgeInsets) -> Bool {
        safe.top >= WontonTheme.size.target + WontonTheme.spacing.s8 && status.title == nil && status.line == nil
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
                insetTop: edgeToEdge ? Double(page.fullscreen ? max(safe.top, topHeight) : topHeight + safe.top) : 0,
                insetBottom: edgeToEdge ? Double(bottom + safe.bottom) : 0,
                safeArea: TabSafeArea(
                    top: edgeToEdge ? Double(safe.top) : 0, right: edgeToEdge ? Double(safe.trailing) : 0,
                    bottom: edgeToEdge ? Double(safe.bottom) : 0, left: edgeToEdge ? Double(safe.leading) : 0
                )
            ), textScale: textScale
        )
    }

    private var topBar: some View {
        VStack(spacing: 0) {
            ZStack {
                Text(status.title ?? page.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                    .padding(.horizontal, WontonTheme.size.target * 2)
                    .accessibilityAddTraits(.isHeader)
                HStack(spacing: 0) {
                    if session.canGoBack {
                        WontonIconButton(icon: .back, label: TabText.back) { session.webView?.goBack() }
                    }
                    Spacer(minLength: 0)
                    if let takeover { TakeoverButton(takeover: takeover) }
                    WontonIconButton(icon: .expand, label: TabText.fullscreen) {
                        tabs.send(.enterFullscreen(tab: tab))
                    }
                    if isOverlay {
                        WontonIconButton(icon: .close, label: TabText.close) {
                            if let close { close() } else { tabs.send(.closeTabOverlay) }
                        }
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.s4)
            }
            .frame(minHeight: WontonTheme.size.topBar)
            if let line = status.line {
                line.padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
            }
        }
        .background(.ultraThinMaterial)
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
                        session?.problem = Problem(title: TabText.choose("打不开这个链接", "Could not open this link"))
                    }
                }
            case let .draftMessage(text): draft(text)
            }
        }
    }
}

private struct EdgeSampling: Equatable {
    let edges: Set<Edge>
    let visible: Bool
    let webView: ObjectIdentifier?
}
