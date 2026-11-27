import AppIntents
import SwiftUI
import WontonClient

struct MainView: View {
    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    let appearance: AppearanceModel
    let lock: AppLockModel
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
    var showsApps: Bool { pages.viewModel.showsApps }
    @EnvironmentObject private var orientation: FullscreenOrientation
    @State var onboardingGateway: TabChoice?
    @State var showsSidebar = false
    @State var showsMyApps = false
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    var usesTwoColumns: Bool {
        horizontalSizeClass == .regular && isPaired && !home.servers.adding && onboarding.viewModel.step == .finished
    }
    @State var showsPanel = false
    /// 没配对时是根页面（网格或配对页）的路径；配对后是“全部应用”那一层的路径，空就是没开（见 `HomeView.layered`）。
    @State var appsPath = NavigationPath()
    /// 插件页层的路径，和核心的页面栈一一对应；左边缘右滑是系统的交互式返回。
    @State var pagePath: [TabRef] = []
    @State private var addingReturn: (serverID: String?, settingsModal: Bool)?

    init(chat: ChatModel, home: HomeModel, plugins: PluginsModel, appearance: AppearanceModel,
         lock: AppLockModel, navigation: AppNavigation, devWebURL: URL?) {
        self.chat = chat
        self.home = home
        self.plugins = plugins
        self.appearance = appearance
        self.lock = lock
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
        .overlay(alignment: .bottom) {
            AskFailureBar(model: chat)
                .padding(.bottom, WontonTheme.size.bottomBar + WontonTheme.spacing.s12)
        }
        #if DEBUG
        .overlay(alignment: .topLeading) { KeyHandoffTestState(model: chat) }
        #endif
        .modifier(ChatDialogs(model: chat))
        .modifier(DeviceVerificationDialogs(model: chat, suppressed: verificationSurfacesOpen))
        .modifier(TabMediaDialogs(model: pages))
        .modifier(KeyHandoffCover(model: chat) {
            pages.open(.chat)
            chat.path = [.chatBackup]
        })

        // 系统 sheet（11-27 主人）：往下拉跟手关，列表在顶上时下拉才关；关闭在右上角（wontonSheetScreen）。
        .sheet(isPresented: $showsSettings) {
            // 参数多，闭包先写成带类型的局部量，不写时整条表达式类型检查会超时。
            let openPlugin: (PluginEntry) -> Void = { showsSettings = false; open($0) }
            let checkInstalled: () -> Void = { showsSettings = false; pages.open(.apps) }
            // 设置里的应用行进了详情后，“打开”和“查看已安装”都要先收起设置，再走主界面这一层。
            SettingsTab(model: settings, appearance: appearance, appLock: lock, chat: chat,
                        home: home, plugins: plugins, open: openPlugin,
                        checkInstalled: checkInstalled, push: navigation.push, isModal: true)
        }
        .wontonDialog(problem: pages.rejection, isPresented: Binding(
            get: { pages.rejection != nil }, set: { if !$0 { pages.rejection = nil } }
        ))
        .onChange(of: pages.viewModel.stack, initial: true) { _, stack in
            let stackTabs = stack.map(\.tab)
            // 关掉的页面按关闭先后记着，最多留 4 个（每个 WebView 一份渲染进程，个数是内存折中）。
            let closed = visited.subtracting(stackTabs)
            visited = Set(stackTabs)
            // widget 页不保活：关掉就是挂断，WebView 跟着释放（ADR-0096）。
            recentPages.append(contentsOf: closed.filter { !$0.isWidget })
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
        .environment(pages)
        .task { await navigation.push.start() }
        .task {
            // UI 测试不必经系统“在云吞中打开?”：直接走快捷指令同一条 askAgent。
            guard ProcessInfo.processInfo.environment["WONTON_UI_TEST_ASK_ON_LAUNCH"] == "1" else { return }
            try? await navigation.askAgent("")
        }
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
        // 根页面和“全部应用”那一层共用一条路径，配对状态一变含义就换了。
        .onChange(of: isPaired) { _, _ in closeApps() }
        .onChange(of: pages.viewModel.showsApps, initial: true) { _, shown in
            if shown && isPaired && appsPath.isEmpty {
                appsPath = NavigationPath([AppsRoute.launcher])
            } else if !shown && !appsPath.isEmpty {
                appsPath = NavigationPath()
            }
        }
        .onChange(of: appsPath.count) { _, count in
            if isPaired && count == 0 && pages.viewModel.showsApps {
                pages.closeCurrent()
            }
        }
        .environment(plugins)
        .environment(\.wontonBarStatus, barStatus)
    }

    /// 服务器列表变化时的界面善后：换服务器清聊天导航；“添加服务器”盖住设置，取消后回设置；
    /// 服务器清空时根页面换成没配对的那一个，“全部应用”那一层跟着收起。

    private func updatePairingPresentation(from old: ServersViewModel, to new: ServersViewModel) {
        if old.servers.first(where: { $0.current })?.id != new.servers.first(where: { $0.current })?.id {
            chat.path = []
            chat.visiting = nil
            closeApps()
        }
        if new.adding && !old.adding {
            addingReturn = (old.servers.first(where: { $0.current })?.id, showsSettings)
            showsSettings = false
            onboardingGateway = nil
            // 配对页在根页面上，“全部应用”那一层盖着它。
            closeApps()
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
            closeApps()
        }
    }

    /// 根页面：配对后是聊天（下拉面板在它上面），没配对是应用网格（配对页也在它上面）。
    /// 配对失败还没走开时也算配对中：失败面只在配对页上，切走就丢了总耗时和诊断。
    @ViewBuilder var rootContent: some View {
        if showsPairingPage {
            NavigationStack {
                if home.servers.adding {
                    // 添加服务器有自己的“取消”那一条，不带侧边栏。
                    PairingView(model: home)
                        .toolbar(.hidden, for: .navigationBar)
                } else {
                    // 没配对时配对失败从 HomeView 的配对页接手过来：照那一页留着侧边栏按钮，失败了也能去别的入口。
                    PairingView(model: home)
                        .wontonPairingRootScreen(
                            String(localized: .Pairing.pairingTitle),
                            hasActions: false
                        ) { EmptyView() }
                }
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
                    .environment(\.wontonOpenApps) { openApps() }
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
            draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
            select: { pages.send(.pages(action: .selectPanelPage(scope: .main, tab: $0))) },
            remove: { pages.send(.pages(action: .removePanelPage(scope: .main, tab: $0))) },
            move: { pages.send(.pages(action: .movePanelPage(scope: .main, tab: $0, index: UInt32($1)))) },
            content: { ChatListView(model: chat, plugins: plugins, listOnly: listOnly) }
        )
    }

    private var pendingElsewhere: [PendingConfirmation] {
        let shown: String? = if pages.showsPluginPage { nil } else { chat.shownRoom(in: .chat) }
        if !pages.showsPluginPage, chat.path.last == .confirmations { return [] }
        return chat.viewModel.pendingConfirmations.filter { $0.roomId == nil || shown != $0.roomId }
    }

    /// “添加设备”和聊天备份页正开着：这两处不管什么状态都在页面里显示确认流程（ADR-0046），所以页面开着就不弹窗。
    /// 页面的显示和这里的压制用同一条判断，页面改了要一起改。
    private var verificationSurfacesOpen: Bool {
        settings.path.contains(.invite) || settings.path.contains(.chatBackup) || chat.path.contains(.chatBackup)
    }

    var isPaired: Bool { home.viewModel.status != .notConfigured }

    /// 配对页正在顶上：正在添加，或者添加已结束但失败面还开着（失败面只在配对页上）。
    var showsPairingPage: Bool {
        if home.servers.adding { return true }
        if case .failed = home.viewModel.pairing { return true }
        return false
    }

    private var onboardingGatewayPage: TabPage? {
        guard let choice = onboardingGateway else { return nil }
        return pages.viewModel.stack.first(where: { $0.tab == choice.tab })?.page
    }
}

extension MainView {
    @ViewBuilder fileprivate var notices: some View {
        if let problem = pages.viewModel.problem {
            ProblemRow(problem: problem.problem)
                .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
        if !pages.viewModel.uninstalled.isEmpty {
            HStack {
                Text(pages.viewModel.uninstalled.map(TabText.uninstalled).joined(separator: "\n"))
                    .wontonText(WontonTheme.typography.supporting)
                Spacer(minLength: 0)
                WontonIconButton(icon: .close, label: TabText.close) {
                    pages.send(.pages(action: .dismissUninstalledTabs))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }

    /// 连接状态条：拆出计算属性，整条 body 的类型检查超时（Swift 编译器限制）。
    fileprivate var barStatus: WontonBarStatus {
        WontonBarStatus(home: home.viewModel, recovered: home.viewModel.justRecovered) {
            ConnectionLine(notice: $0, retry: { home.send(.home(action: .retryConnection)) }, open: showServerRoute)
        }
    }
}
