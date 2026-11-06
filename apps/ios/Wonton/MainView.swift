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
    var tabs: TabsModel { navigation.tabs }
    var onboarding: OnboardingModel { navigation.onboarding }
    @State var settings: SettingsModel
    @State private var visited: Set<TabRef> = []
    @State private var isKeyboardVisible = false
    @State var showsTabSettings = false
    @State var showsSettings = false
    @State var pendingSettingsPresentation = false
    @EnvironmentObject private var orientation: FullscreenOrientation
    @State var onboardingGateway: TabChoice?

    init(chat: ChatModel, home: HomeModel, plugins: PluginsModel, appearance: AppearanceModel,
         navigation: AppNavigation, devWebURL: URL?) {
        self.chat = chat
        self.home = home
        self.plugins = plugins
        self.appearance = appearance
        self.devWebURL = devWebURL
        self.navigation = navigation
        _settings = State(initialValue: SettingsModel(core: chat.core))
    }

    var body: some View {
        OnboardingGate(model: onboarding, plugins: plugins, isPaired: isPaired,
                       finished: finishOnboarding, openGateway: openOnboardingGateway, content: {
            VStack(spacing: 0) {
                if !selectedFullscreen, !pendingElsewhere.isEmpty {
                    PendingCardBanner(cards: pendingElsewhere, open: openCardRoom)
                }
                notices
                pages
            }
        })
        .overlay { if chat.isOpeningChat { ProgressView() } }
        .statusBarHidden(selectedFullscreen && tabs.viewModel.overlay == nil)
        .modifier(ChatDialogs(model: chat))
        .fullScreenCover(isPresented: overlayPresented, onDismiss: presentPendingSettings) { overlay }
        .fullScreenCover(
            item: $onboardingGateway, onDismiss: {
                onboarding.send(.refreshOnboarding)
                presentPendingSettings()
            }, content: { choice in
                if let page = onboardingGatewayPage {
                    TabPageView(tabs: tabs, home: home, plugins: plugins, appearance: appearance,
                                tab: choice.tab, page: page, isOverlay: true, draft: receiveDraft, open: open,
                                pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom, close: {
                        tabs.send(.closeTabOverlay)
                        onboardingGateway = nil
                    })
                }
            }
        )
        .fullScreenCover(isPresented: $showsSettings) {
            SettingsTab(model: settings, appearance: appearance, chat: chat,
                        home: home, plugins: plugins, isModal: true)
        }
        .fullScreenCover(isPresented: $showsTabSettings, onDismiss: presentPendingSettings) {
            NavigationStack { TabSettingsView(plugins: plugins).wontonModalScreen(TabText.bottomBar) }
        }
        .wontonDialog(problem: tabs.rejection, isPresented: Binding(
            get: { tabs.rejection != nil }, set: { if !$0 { tabs.rejection = nil } }
        ))
        .onChange(of: tabs.viewModel.selected, initial: true) { _, selected in visited.insert(selected) }
        .onChange(of: visibleFullscreen, initial: true) { _, fullscreen in
            orientation.update(fullscreen: fullscreen) { tabs.rejection = $0.problem }
        }
        .onChange(of: chat.openingFailed) { _, failed in
            if failed { chat.openingFailed = false; tabs.select(.chat) }
        }
        .task { await tabs.observe() }
        .task { await home.observe() }
        .task { await plugins.observe() }
        .task { await chat.observe() }
        .task { await chat.files.observe() }
        .task { await settings.observe() }
        .task {
            for await network in DeviceNetwork.changes() { home.send(.networkChanged(network: network)) }
        }
        .environment(tabs)
        .environment(onboarding)
        .onChange(of: onboarding.viewModel.step) { _, step in
            if step == .profile { showsSettings = false; showsTabSettings = false }
        }
        .onChange(of: onboardingGatewayPage) { _, page in
            if page == nil { onboardingGateway = nil }
        }
        .onOpenURL(perform: receiveAppURL)
        .onChange(of: navigation.presentationRevision, initial: true) { _, _ in applyAppNavigation() }
        .onChange(of: tabs.viewModel.items, initial: true) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .onChange(of: tabs.viewModel.candidates) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .onChange(of: onboarding.viewModel.step) { _, _ in WontonShortcuts.updateAppShortcutParameters() }
        .environment(plugins)
        .environment(\.wontonBarStatus, WontonBarStatus(notice: home.viewModel.notice) {
            ConnectionLine(notice: $0, retry: { home.send(.retryConnection) }, open: showServerRoute)
        })
    }

    private var pages: some View {
        TabView(selection: Binding(get: { tabs.viewModel.selected }, set: { tabs.select($0) })) {
            ForEach(tabs.viewModel.items, id: \.tab) { item in
                page(item).tag(item.tab).toolbar(.hidden, for: .tabBar)
            }
        }
        .environment(\.wontonTabBar, isKeyboardVisible ? nil : AnyView(bottomBar))
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            isKeyboardVisible = true
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
            isKeyboardVisible = false
        }
    }

    @ViewBuilder private func page(_ item: TabItem) -> some View {
        switch item.tab {
        case .chat: ChatListView(model: chat, plugins: plugins)
        case .apps:
            HomeView(
                model: home, plugins: plugins, chat: chat, settings: settings, open: open,
                pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
                showSettings: { settings.path = []; showsSettings = true },
                showServer: { showServerRoute(.server) }, settingsPresented: showsSettings, devWebURL: devWebURL
            )
        case .settings:
            SettingsTab(model: settings, appearance: appearance, chat: chat, home: home, plugins: plugins,
                        isActive: tabs.viewModel.selected == .settings && !showsSettings)
        case .plugin:
            if visited.contains(item.tab), let page = item.page { pluginPage(item.tab, page: page, overlay: false) }
        }
    }

    @ViewBuilder private var overlay: some View {
        if let item = tabs.viewModel.overlay, let page = item.page {
            VStack(spacing: 0) {
                if !page.fullscreen, !chat.viewModel.pendingConfirmations.isEmpty {
                    PendingCardBanner(cards: chat.viewModel.pendingConfirmations, open: openCardRoom)
                }
                pluginPage(item.tab, page: page, overlay: true)
            }
            .statusBarHidden(page.fullscreen)
        }
    }

    private func pluginPage(_ tab: TabRef, page: TabPage, overlay: Bool) -> some View {
        TabPageView(
            tabs: tabs, home: home, plugins: plugins, appearance: appearance,
            tab: tab, page: page, isOverlay: overlay, draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom
        )
        .id(tab)
    }

    private var selectedFullscreen: Bool {
        tabs.viewModel.items.first { $0.tab == tabs.viewModel.selected }?.page?.fullscreen == true
    }

    private var visibleFullscreen: Bool {
        tabs.viewModel.overlay?.page?.fullscreen ?? selectedFullscreen
    }

    @ViewBuilder private var notices: some View {
        if let problem = tabs.viewModel.problem {
            ProblemRow(problem: Problem(title: TabText.configurationFailed, detail: problem.message))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
        if !tabs.viewModel.uninstalled.isEmpty {
            HStack {
                Text(tabs.viewModel.uninstalled.map(TabText.uninstalled).joined(separator: "\n"))
                    .wontonText(WontonTheme.typography.supporting)
                Spacer(minLength: 0)
                WontonIconButton(icon: .close, label: TabText.close) { tabs.send(.dismissUninstalledTabs) }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }

    private var pendingElsewhere: [PendingConfirmation] {
        let shown: String?
        if tabs.viewModel.overlay != nil {
            shown = nil
        } else if case .plugin = tabs.viewModel.selected {
            shown = nil
        } else {
            shown = chat.shownRoom(in: nativeTab)
        }
        return chat.viewModel.pendingConfirmations.filter { shown != $0.roomId }
    }

    var nativeTab: AppTab {
        switch tabs.viewModel.selected {
        case .chat: .chat
        case .apps, .plugin: .apps
        case .settings: .settings
        }
    }

    private var isPaired: Bool { home.viewModel.status != .notConfigured }
    var overlayPresented: Binding<Bool> {
        Binding(get: { tabs.viewModel.overlay != nil && onboardingGateway == nil },
                set: { if !$0 { tabs.send(.closeTabOverlay) } })
    }

    private var onboardingGatewayPage: TabPage? {
        guard let choice = onboardingGateway else { return nil }
        if let item = tabs.viewModel.items.first(where: { $0.tab == choice.tab }) { return item.page }
        guard tabs.viewModel.overlay?.tab == choice.tab else { return nil }
        return tabs.viewModel.overlay?.page
    }

}

extension MainView {

    private var bottomBar: some View {
        WontonBottomBar(
            items: tabs.viewModel.items.map { item in
                .init(
                    tab: item.tab, label: item.title,
                    icon: AnyView(TabIconView(item: item, selected: false, plugins: plugins)),
                    selectedIcon: AnyView(TabIconView(item: item, selected: true, plugins: plugins)),
                    badge: item.badge, isDimmed: item.page?.dimmed == true,
                    opensFullscreen: item.page?.opensFullscreen
                )
            },
            labels: tabs.viewModel.labels,
            selection: Binding(get: { tabs.viewModel.selected }, set: { tabs.select($0) }),
            reselect: { tabs.select($0) }, longPress: { _ in showsTabSettings = true },
            enterFullscreen: { tabs.send(.enterFullscreen(tab: $0)) },
            setAlwaysFullscreen: { tabs.send(.setTabDisplay(tab: $0, display: $1 ? .fullscreen : .standard)) }
        )
    }
}
