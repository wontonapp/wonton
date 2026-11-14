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
    let openCardRoom: (String?) -> Void
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
    @State private var disclosedURL: String?
    @State private var disclosedProxy: ProxySettings?
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase
    @ScaledMetric(relativeTo: .body) private var textScale: CGFloat = 1

    init(
        pages: PagesModel, home: HomeModel, plugins: PluginsModel, chat: ChatModel, appearance: AppearanceModel,
        tab: TabRef, placed: PlacedTab, store: TabWebViewStore, page: TabPage,
        draft: @escaping (String) -> Void, open: @escaping (PluginEntry) -> Void,
        pendingCards: [PendingConfirmation], openCardRoom: @escaping (String?) -> Void,
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
        .onChange(of: scenePhase) { _, phase in
            // 第一版离开通话页就挂断（ADR-0096 第 3 节）：App 退到后台，widget 页跟着关。
            if phase == .background, tab.isWidget { closePage() }
        }
        .onAppear { notifySeen(); isVisible = true }
        .onDisappear {
            isVisible = false
            if panel == nil { takeover?.end() }
        }
        .sheet(isPresented: $showsDetails) {
            if case let .bot(bot, _) = tab {
                NavigationStack {
                    BotProfileView(chat: chat, plugins: plugins, botRef: bot) { showsDetails = false }
                }
            } else if let pluginId {
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
                if blocksBotPage {
                    ProblemView(problem: Problem(title: blockedBotText), retry: session.retry)
                } else if requiresDisclosure {
                    BotWebDisclosure(url: page.url, proceed: {
                        disclosedURL = page.url
                        disclosedProxy = home.viewModel.proxy
                        updatePermittedSession(inputs)
                    }, cancel: closePage)
                } else {
                    if let webView = session.webView, sourceAvailable {
                        TabWebView(webView: webView, obscured: obscured(safe: safe))
                            .id(ObjectIdentifier(webView))
                            .ignoresSafeArea(.container)
                    }
                    TabPageStateView(
                        page: page, session: session, connected: connected,
                        shareProblem: plugins.viewModel.sharedProblem(for: tab), isPanel: panel != nil,
                        details: { showsDetails = true }, remove: closePage
                    )
                    .ignoresSafeArea(.container)
                }
                chrome(safe: safe)
            }
            .background { Color(colors.canvas).ignoresSafeArea(.container) }
            .background {
                if let panel {
                    PanelPagePull(
                        enabled: panel.pagePull && isVisible, webView: session.webView, edge: panel.pageEdge,
                        zone: PagePullZone(obscured: obscured(safe: safe)),
                        changed: panel.pageChanged, ended: panel.pageEnded, cancelled: panel.pageCancelled
                    )
                }
            }
            .task(id: inputs) { updatePermittedSession(inputs) }
            .onChange(of: blocksBotPage) { updatePermittedSession(inputs) }
            .onChange(of: requiresDisclosure) { updatePermittedSession(inputs) }
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
            if callRelayed {
                CallRelayHint()
                    .padding(.top, WontonTheme.spacing.s8)
                    .padding(.leading, safe.leading + WontonTheme.spacing.s8)
                    .padding(.trailing, safe.trailing + WontonTheme.spacing.s8)
            }
            Spacer(minLength: 0)
        }
        .ignoresSafeArea(.container)
    }

    /// widget 页的媒体走插件的端口映射；映射只连上了公共中继时核心给提醒的类别（ADR-0096 第 4 节）。
    private var callRelayed: Bool {
        guard case let .widget(pluginId, _) = tab else { return false }
        return plugins.viewModel.portMappings.first { $0.pluginId == pluginId }?.warning == .publicRelayBandwidth
    }

    private func closePage() {
        if let remove {
            remove()
        } else if let close {
            close()
        } else {
            pages.send(.closePage)
        }
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

}

private extension TabPageView {
    /// Only Bot-bound entries need this disclosure; ordinary plugin pages retain their existing behavior.
    private var requiresDisclosure: Bool {
        guard isBotPage, let url = page.url else { return false }
        return disclosedURL != url || disclosedProxy != home.viewModel.proxy
    }

    private var pageBotRef: BotRef? {
        switch tab {
        case let .bot(bot, _): return bot
        case let .plugin(id, _), let .pluginPath(id, _):
            guard plugins.viewModel.installed?.plugins.contains(where: {
                $0.id == id && ($0.botUserId != nil || $0.chat != nil)
            }) == true else { return nil }
            return BotRef(pluginId: id, bot: "default")
        case .chat, .apps, .url, .sharedApp, .uiResource, .widget: return nil
        }
    }

    private var pageBot: BotView? {
        guard let pageBotRef else { return nil }
        return chat.viewModel.bots.first { $0.bot == pageBotRef }
    }

    private var verifiedBotPage: Bool {
        guard let ref = pageBotRef, let bot = pageBot, let web = bot.web,
              case let .ready(metadata) = web, metadata.bot == ref,
              let origin = webOrigin(metadata.url), origin == webOrigin(page.url) else { return false }
        return true
    }

    private var blocksBotPage: Bool { isBotPage && !verifiedBotPage }

    private var blockedBotText: String {
        if let web = pageBot?.web, case let .unavailable(reason) = web { return reason.profileText }
        return String(localized: .BotProfile1.botProfileWebUnknown)
    }

    private func updatePermittedSession(_ inputs: TabWebInputs) {
        guard !blocksBotPage, !requiresDisclosure else {
            session.withhold()
            return
        }
        updateSession(inputs)
    }

    private func webOrigin(_ value: String?) -> String? {
        guard let value, var url = URLComponents(string: value), let scheme = url.scheme?.lowercased(),
              let host = url.host?.lowercased() else { return nil }
        url.scheme = scheme
        url.host = host
        if (scheme == "http" && url.port == 80) || (scheme == "https" && url.port == 443) { url.port = nil }
        url.path = ""
        url.query = nil
        url.fragment = nil
        url.user = nil
        url.password = nil
        return url.string
    }

    private var isBotPage: Bool {
        switch tab {
        case .bot: return true
        case .plugin, .pluginPath:
            guard pageBotRef != nil else { return false }
            if let web = pageBot?.web, case .unavailable(.noWebui) = web { return false }
            return true
        case .chat, .apps, .url, .sharedApp, .uiResource, .widget: return false
        }
    }

    private var takeover: PageTakeover? {
        if case .bot = tab { return nil }
        guard let pluginId, let plugin = plugins.viewModel.installed?.plugins.first(where: { $0.id == pluginId }),
              plugin.declaresTools else { return nil }
        return PageTakeover(pluginId: pluginId, current: plugins.viewModel.takeover, send: plugins.send)
    }

    /// 网页被浮在上面的东西挡住的上下两条。整页铺满整个屏幕（ADR-0095），顶上是状态栏加胶囊那一行：
    /// 页面的背景照样画到屏幕顶，固定在顶上的按钮和滚动到头的位置让到胶囊下面，不然压在胶囊底下点不到。
    /// 面板里的页面照面板的布局让出上下两条（ADR-0052）。
    private func obscured(safe: EdgeInsets) -> UIEdgeInsets {
        if let panel {
            return UIEdgeInsets(top: panel.insetTop, left: 0, bottom: panel.insetBottom, right: 0)
        }
        return UIEdgeInsets(top: safe.top + TabPageCapsule.rowHeight, left: 0, bottom: 0, right: 0)
    }

    private func inputs(safe: EdgeInsets) -> TabWebInputs {
        let bottom = panel?.insetBottom ?? 0
        let top = panel?.insetTop ?? pageInsetTop(safe: safe)
        return TabWebInputs(
            page: page, proxy: sourceProxy, connected: connected,
            appearance: appearance.viewModel,
            environment: TabEnvironment(
                locale: AppLanguage.current,
                timeZone: TimeZone.current.identifier, platform: .ios, dark: colorScheme == .dark,
                insetTop: Double(top), insetBottom: Double(bottom + (panel == nil ? safe.bottom : 0)),
                safeArea: TabSafeArea(
                    top: Double(pageSafeTop(safe: safe)), right: Double(safe.trailing),
                    bottom: Double(safe.bottom), left: Double(safe.leading)
                )
            ), textScale: textScale
        )
    }

    /// 整页经桥报给页面的顶部让位（`--wonton-inset-top`）。iOS 26 起 WebKit 已经按 `obscured` 把页面的布局
    /// 让到胶囊下面，再报一份页面会让两次，所以报 0；更早的系统没有这个接口，把状态栏加胶囊那一行报给页面，
    /// 适配过桥的页面自己让。
    private func pageInsetTop(safe: EdgeInsets) -> CGFloat {
        if #available(iOS 26, *) { return 0 }
        return safe.top + TabPageCapsule.rowHeight
    }

    /// 桥的 `safeAreaInsets.top`：同 `pageInsetTop`，iOS 26 起整页的顶部安全区已经被 WebKit 让掉。
    private func pageSafeTop(safe: EdgeInsets) -> CGFloat {
        if #available(iOS 26, *), panel == nil { return 0 }
        return safe.top
    }

    /// 会话被释放过（`TabWebViewStore.retain` 放掉的）：重新要一份并加载。
    /// 面板只留当前页和左右各一页（ADR-0052）。
    private func keepSessionAlive(_ inputs: TabWebInputs, when keep: Bool) {
        if keep, session.webView == nil { revive(inputs) }
    }

    private func revive(_ inputs: TabWebInputs) {
        session = store.session(for: placed, core: pages.core)
        updatePermittedSession(inputs)
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
