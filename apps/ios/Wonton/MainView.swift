import AppIntents
import SwiftUI
import WontonClient

struct MainView: View {
    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    let appearance: AppearanceModel
    let devWebURL: URL?
    let navigation: AppNavigation
    var pages: PagesModel { navigation.pages }
    var onboarding: OnboardingModel { navigation.onboarding }
    @State var settings: SettingsModel
    /// 打开过的插件页（含最近关掉留保活的）；保活清单见 recentPages。
    @State private var visited: Set<TabRef> = []
    /// 最近关掉的插件页：最多留 4 个实例（内存考虑），重开不重载。
    @State private var recentPages: [TabRef] = []
    @State var showsSettings = false
    @State var pendingSettingsPresentation = false
    /// “全部应用”网格这一层：配对后的根页面是聊天，网格从侧边栏进。
    @State var showsApps = false
    @EnvironmentObject private var orientation: FullscreenOrientation
    @State var onboardingGateway: TabChoice?
    @State var showsSidebar = false
    @State var showsMyApps = false
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    var usesTwoColumns: Bool {
        horizontalSizeClass == .regular && isPaired && !home.servers.adding && onboarding.viewModel.step == .finished
    }
    @State var showsPanel = false
    @State var appsPath = NavigationPath()
    /// 插件页层的路径，和核心的页面栈一一对应；左边缘右滑是系统的交互式返回。
    @State var pagePath: [TabRef] = []
    @State private var addingReturn: (serverID: String?, settingsModal: Bool)?

    init(chat: ChatModel, home: HomeModel, plugins: PluginsModel, appearance: AppearanceModel,
         navigation: AppNavigation, devWebURL: URL?) {
        self.chat = chat
        self.home = home
        self.plugins = plugins
        self.appearance = appearance
        self.navigation = navigation
        self.devWebURL = devWebURL
        _settings = State(initialValue: SettingsModel(core: chat.core))
    }

    var body: some View {
        @Bindable var navigation = navigation
        return adaptiveContent
        .onPreferenceChange(ChatPanelOpenKey.self) { showsPanel = $0 }
        .onChange(of: horizontalSizeClass) { _, _ in showsSidebar = false }
        .overlay { if chat.isOpeningChat { ProgressView() } }
        .modifier(ChatDialogs(model: chat))
        .modifier(DeviceVerificationDialogs(model: chat, suppressed: verificationSurfacesOpen))
        .modifier(KeyHandoffCover(model: chat) {
            pages.open(.chat)
            chat.path = [.chatBackup]
        })
        .fullScreenCover(isPresented: $showsApps, onDismiss: presentPendingSettings) {
            appsGrid
        }

        .fullScreenCover(isPresented: $showsSettings) {
            SettingsTab(model: settings, appearance: appearance, chat: chat,
                        home: home, plugins: plugins, isModal: true)
        }
        .wontonDialog(problem: pages.rejection, isPresented: Binding(
            get: { pages.rejection != nil }, set: { if !$0 { pages.rejection = nil } }
        ))
        .onChange(of: pages.viewModel.stack, initial: true) { _, stack in
            let stackTabs = stack.map(\.tab)
            // 关掉的页面按关闭先后记着，最多留 4 个（每个 WebView 一份渲染进程，个数是内存折中）。
            let closed = visited.subtracting(stackTabs)
            visited = Set(stackTabs)
            recentPages.append(contentsOf: closed)
            recentPages = Array(recentPages.suffix(4))
            visited.formUnion(recentPages)
            pages.setKeepAlive(recentPages)
            // 核心弹了栈（胶囊关页、页面被卸载）：路径跟着对齐。系统返回的弹栈由 pagePath 的 onChange 发。
            pagePath = stackTabs
        }
        .onChange(of: pagePath) { _, path in
            // 主人在页面层做了系统返回：把弹栈交给核心，核心的订阅再把两边对齐。
            if path.count < pages.viewModel.stack.count { pages.closeCurrent() }
        }
        .onChange(of: pages.showsPluginPage, initial: true) { _, shown in
            // 页面默认全屏：横屏只在插件页上放开，回根页面收回来。
            orientation.update(fullscreen: shown) { pages.rejection = $0.problem }
        }
        .onChange(of: chat.openingFailed) { _, failed in
            if failed { chat.openingFailed = false; pages.open(.chat) }
        }
        .task { await pages.observe() }
        .task { await home.observe() }
        .task { await home.observeServers() }
        .task { await plugins.observe() }
        .task { await chat.observe() }
        .task { await chat.files.observe() }
        .task { await settings.observe() }
        .task {
            for await network in DeviceNetwork.changes() { home.send(.networkChanged(network: network)) }
        }
        .environment(pages)
        .environment(onboarding)
        .environment(\.panelDeps, PanelDependencies(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            draft: receiveDraft, open: open, pendingCards: chat.viewModel.pendingConfirmations,
            openCardRoom: openCardRoom
        ))
        .onChange(of: onboarding.viewModel.step) { _, step in
            if step == .profile { showsSettings = false }
        }
        .onChange(of: onboardingGatewayPage) { _, page in
            if page == nil { onboardingGateway = nil }
        }
        .sheet(item: $navigation.safari) { target in
            SafariView(url: target.url).ignoresSafeArea()
        }
        .onOpenURL(perform: receiveAppURL)
        .modifier(ShareInPresenter(model: navigation.shareIn, chat: chat, plugins: plugins,
                                   openRoom: navigation.openSharedRoom))
        .onChange(of: navigation.presentationRevision, initial: true) { _, _ in applyAppNavigation() }
        .onChange(of: pages.viewModel.stack, initial: true) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .onChange(of: pages.viewModel.myAppCandidates) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .onChange(of: onboarding.viewModel.step) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .onChange(of: home.servers) { old, new in updatePairingPresentation(from: old, to: new) }
        .environment(plugins)
        .environment(\.wontonBarStatus, barStatus)
    }

    /// 服务器列表变化时的界面善后：换服务器清聊天导航；“添加服务器”盖住设置，取消后回设置；
    /// 服务器清空时回应用网格。

    private func updatePairingPresentation(from old: ServersViewModel, to new: ServersViewModel) {
        if old.servers.first(where: { $0.current })?.id != new.servers.first(where: { $0.current })?.id {
            chat.path = []
            chat.visiting = nil
        }
        if new.adding && !old.adding {
            addingReturn = (old.servers.first(where: { $0.current })?.id, showsSettings)
            showsSettings = false
            onboardingGateway = nil
            pendingSettingsPresentation = false
        } else if old.adding && !new.adding {
            if let target = addingReturn,
               new.servers.first(where: { $0.current })?.id == target.serverID {
                showsSettings = true
            }
            addingReturn = nil
        }
        if !new.adding, new.servers.isEmpty, !old.servers.isEmpty {
            showsSettings = false
            settings.path = []
            showsApps = true
        }
    }

    /// 根页面：配对后是聊天（下拉面板在它上面），没配对是应用网格（配对页也在它上面）。
    @ViewBuilder var rootContent: some View {
        if home.servers.adding {
            NavigationStack {
                PairingView(model: home)
                    .toolbar(.hidden, for: .navigationBar)
            }
            .wontonDialog(problem: home.rejection, isPresented: Binding(
                get: { home.rejection != nil }, set: { if !$0 { home.rejection = nil } }
            ))
        } else {
            OnboardingGate(model: onboarding, plugins: plugins, isPaired: isPaired,
                           finished: finishOnboarding, openGateway: openOnboardingGateway,
                           close: { showsSettings = true }, content: {
                VStack(spacing: 0) {
                    if !pages.showsPluginPage, !pendingElsewhere.isEmpty {
                        PendingCardBanner(cards: pendingElsewhere, open: openCardRoom)
                    }
                    notices
                    ZStack {
                        if isPaired {
                            chatRoot
                        } else {
                            HomeView(
                                model: home, plugins: plugins, chat: chat, settings: settings, open: open,
                                pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
                                showSettings: { settings.path = []; showsSettings = true },
                                showServer: { showServerRoute(.server) }, settingsPresented: showsSettings,
                                devWebURL: devWebURL, path: $appsPath
                            )
                        }
                    }
                    .environment(\.wontonOpenSidebar, usesTwoColumns ? nil : { showsSidebar = true })
                        .environment(\.wontonOpenApps) { showsApps = true }
                }
            })
        }
    }

    /// 配对后的根页面：聊天列表加下拉面板。
    @ViewBuilder private var chatRoot: some View {
        if usesTwoColumns {
            ChatDetailView(model: chat, plugins: plugins)
        } else {
            chatListPanel(listOnly: false)
        }
    }

    func chatListPanel(listOnly: Bool) -> some View {
        ChatPanelHost(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            scope: .main, edge: .top, items: pages.viewModel.panel, selected: pages.viewModel.panelSelected,
            blocked: false, draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
            select: { pages.send(.selectPanelPage(scope: .main, tab: $0)) },
            remove: { pages.send(.removePanelPage(scope: .main, tab: $0)) },
            move: { pages.send(.movePanelPage(scope: .main, tab: $0, index: UInt32($1))) },
            pullAnchor: nil,
            content: { ChatListView(model: chat, plugins: plugins, listOnly: listOnly) }
        )
    }

    /// 侧边栏的“全部应用”：独立的一层，配对后才开。
    @ViewBuilder private var appsGrid: some View {
        if isPaired {
            HomeView(
                model: home, plugins: plugins, chat: chat, settings: settings, open: open,
                pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
                showSettings: { settings.path = []; showsSettings = true },
                showServer: { showServerRoute(.server) }, settingsPresented: showsSettings || !isPaired,
                devWebURL: devWebURL, path: $appsPath
            )
        }
    }

    /// 插件页层：系统的 NavigationStack 供给左边缘右滑的交互式返回；路径空时透明且不挡下面的根页面。
    @ViewBuilder var pageStack: some View {
        NavigationStack(path: $pagePath) {
            Color.clear
                .background { PageStackBack() }
                .navigationDestination(for: TabRef.self) { tab in
                    Group {
                        if let open = pages.viewModel.stack.first(where: { $0.tab == tab }) {
                            pluginPage(open)
                        } else {
                            // 核心已把这页弹走：保活的会话还在，等这一帧过渡结束。
                            Color.clear
                        }
                    }
                    .toolbar(.hidden, for: .navigationBar)
                }
        }
        .allowsHitTesting(pagePath.isEmpty ? false : pages.showsPluginPage)
    }

    @ViewBuilder private func pluginPage(_ entry: OpenPage) -> some View {
        TabPageView(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            tab: entry.tab, placed: .page(tab: entry.tab), store: pages.webViews,
            page: entry.page, draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
            close: onboardingGateway?.tab == entry.tab ? { closeGateway(entry.tab) } : nil
        )
        .id(entry.tab)
    }

    private func closeGateway(_ tab: TabRef) {
        if pages.viewModel.stack.last?.tab == tab { pages.closeCurrent() }
        onboardingGateway = nil
    }

    private var pendingElsewhere: [PendingConfirmation] {
        let shown: String? = if pages.showsPluginPage { nil } else { chat.shownRoom(in: .chat) }
        return chat.viewModel.pendingConfirmations.filter { shown != $0.roomId }
    }

    /// “添加设备”和聊天备份页正开着：这两处不管什么状态都在页面里显示确认流程（ADR-0046），所以页面开着就不弹窗。
    /// 页面的显示和这里的压制用同一条判断，页面改了要一起改。
    private var verificationSurfacesOpen: Bool {
        settings.path.contains(.invite) || settings.path.contains(.chatBackup) || chat.path.contains(.chatBackup)
    }

    var isPaired: Bool { home.viewModel.status != .notConfigured }

    private var onboardingGatewayPage: TabPage? {
        guard let choice = onboardingGateway else { return nil }
        return pages.viewModel.stack.first(where: { $0.tab == choice.tab })?.page
    }
}

extension MainView {
    @ViewBuilder fileprivate var notices: some View {
        if let problem = pages.viewModel.problem {
            ProblemRow(problem: Problem(title: TabText.configurationFailed, detail: problem.message))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
        if !pages.viewModel.uninstalled.isEmpty {
            HStack {
                Text(pages.viewModel.uninstalled.map(TabText.uninstalled).joined(separator: "\n"))
                    .wontonText(WontonTheme.typography.supporting)
                Spacer(minLength: 0)
                WontonIconButton(icon: .close, label: TabText.close) { pages.send(.dismissUninstalledTabs) }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }

    /// 连接状态条：拆出计算属性，整条 body 的类型检查超时（Swift 编译器限制）。
    fileprivate var barStatus: WontonBarStatus {
        WontonBarStatus(home: home.viewModel, recovered: home.viewModel.justRecovered) {
            ConnectionLine(notice: $0, retry: { home.send(.retryConnection) }, open: showServerRoute)
        }
    }
}
