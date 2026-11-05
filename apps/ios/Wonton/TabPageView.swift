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
    @State private var session: TabWebSession
    @State private var showsDetails = false
    @State private var isVisible = false
    @State private var topHeight: CGFloat = WontonTheme.size.topBar
    @State private var bottomHeight: CGFloat = 0
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
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String) -> Void
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
        _session = State(initialValue: tabs.webViews.session(for: tab, core: tabs.core))
    }

    var body: some View {
        GeometryReader { geometry in
            let safe = geometry.safeAreaInsets
            let inputs = inputs(safe: safe)
            ZStack {
                if let webView = session.webView {
                    TabWebView(webView: webView)
                    .id(ObjectIdentifier(webView))
                    .padding(.top, edgeToEdge ? 0 : topHeight)
                    .padding(.bottom, edgeToEdge ? 0 : contentBottom)
                    .ignoresSafeArea(.container, edges: edgeToEdge ? .all : [])
                }
                TabPageStateView(
                    page: page, session: session, connected: connected,
                    details: { showsDetails = true },
                    remove: { tabs.send(isOverlay ? .closeTabOverlay : .unpinTab(tab: tab)) }
                )
                .padding(.top, topHeight)
                .padding(.bottom, contentBottom)
                VStack(spacing: 0) {
                    header
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { topHeight = $0 }
                    if page.fullscreen, !pendingCards.isEmpty {
                        PendingCardBanner(cards: pendingCards, open: openCardRoom)
                    }
                    if let takeover { TakeoverNotice(takeover: takeover) }
                    Spacer(minLength: 0)
                    if !page.fullscreen, !isOverlay, let bottomBar {
                        bottomBar
                            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { bottomHeight = $0 }
                    }
                }
            }
            .background(Color(colors.canvas).ignoresSafeArea())
            .background {
                FullscreenExitGesture(
                    enabled: page.fullscreen && isVisible && !showsDetails, webView: session.webView,
                    exit: { tabs.send(.exitFullscreen(tab: tab)) }
                )
            }
            .task(id: inputs) { updateSession(inputs) }
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
                locale: Locale.preferredLanguages.first ?? Locale.current.identifier,
                timeZone: TimeZone.current.identifier, platform: .ios, dark: colorScheme == .dark,
                insetTop: edgeToEdge ? Double(topHeight + safe.top) : 0,
                insetBottom: edgeToEdge ? Double(bottom + safe.bottom) : 0,
                safeArea: TabSafeArea(
                    top: edgeToEdge ? Double(safe.top) : 0, right: edgeToEdge ? Double(safe.trailing) : 0,
                    bottom: edgeToEdge ? Double(safe.bottom) : 0, left: edgeToEdge ? Double(safe.leading) : 0
                )
            ), textScale: textScale
        )
    }

    @ViewBuilder private var header: some View {
        if page.fullscreen {
            TabFullscreenBar(page: page, tab: tab, tabs: tabs, details: { showsDetails = true })
                .background {
                    if !edgeToEdge {
                        Color(uiColor: session.themeColor ?? UIColor(Color(colors.canvas)))
                            .ignoresSafeArea(.container, edges: .top)
                    }
                }
        } else { topBar }
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
                        WontonIconButton(icon: .close, label: TabText.close) { tabs.send(.closeTabOverlay) }
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
