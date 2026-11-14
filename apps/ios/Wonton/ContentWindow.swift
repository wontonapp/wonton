import SwiftUI
import WontonClient

struct ContentWindow: View {
    let target: ContentWindowTarget
    let home: HomeModel
    let plugins: PluginsModel
    let appearance: AppearanceModel
    let navigation: AppNavigation
    @State private var chat: ChatModel
    @State private var pages: PagesModel
    @State private var onboarding: OnboardingModel
    @SceneStorage("content-page-closed") private var closed = false

    init(target: ContentWindowTarget, chat: ChatModel, home: HomeModel, plugins: PluginsModel,
         appearance: AppearanceModel, navigation: AppNavigation) {
        self.target = target
        self.home = home
        self.plugins = plugins
        self.appearance = appearance
        self.navigation = navigation
        let windowChat = ChatModel(core: chat.core, draftStore: chat.draftStore)
        _chat = State(initialValue: windowChat)
        _pages = State(initialValue: PagesModel(core: chat.core))
        _onboarding = State(initialValue: OnboardingModel(core: chat.core))
    }

    var body: some View {
        Group {
            if home.servers.servers.first(where: \.current)?.id != target.server {
                Text(.Sidebar.sidebarWindowDifferentServer)
                    .padding(WontonTheme.spacing.pageInset)
            } else if closed {
                Text(.Sidebar.sidebarWindowClosed)
            } else {
                surface
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .environment(pages)
        .environment(plugins)
        .environment(onboarding)
        .environment(\.wontonBarStatus, barStatus)
        .environment(\.panelDeps, PanelDependencies(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            draft: receiveDraft, open: open, pendingCards: chat.viewModel.pendingConfirmations,
            openCardRoom: openCardRoom
        ))
        .modifier(ChatDialogs(model: chat))
        .task { await chat.observe() }
        .task { await chat.files.observe() }
        .task { await pages.observe() }
        .task { await home.observe() }
        .task { await home.observeServers() }
        .task { await plugins.observe() }
        .task { await onboarding.observe() }
        .task { await appearance.observe() }
        .wontonDialog(problem: pages.rejection, isPresented: Binding(
            get: { pages.rejection != nil }, set: { if !$0 { pages.rejection = nil } }
        ))
    }

    private var barStatus: WontonBarStatus {
        WontonBarStatus(home: home.viewModel, recovered: home.viewModel.justRecovered) {
            ConnectionLine(notice: $0, retry: { home.send(.retryConnection) }, open: { _ in
                navigation.request { try await $0.open(.settings) }
            })
        }
    }

    @ViewBuilder private var surface: some View {
        switch parseAppLink(url: target.link) {
        case let .room(roomId):
            ContentRoom(model: chat, plugins: plugins, roomId: roomId)
        case let .tab(tab):
            ContentWebPage(core: chat.core, tab: tab, pages: pages, home: home, plugins: plugins,
                           chat: chat, appearance: appearance, draft: receiveDraft, open: open,
                           openCardRoom: openCardRoom, closed: { closed = true })
        default:
            Text(.Sidebar.sidebarWindowUnavailable)
        }
    }

    private func openCardRoom(_ roomId: String) {
        navigation.request { try await $0.open(.room(roomId: roomId)) }
    }

    private func receiveDraft(_ text: String) {
        if case let .room(roomId) = parseAppLink(url: target.link),
           chat.viewModel.rooms.first(where: { $0.roomId == roomId })?.isReadOnly == false {
            chat.appendDraft(text, to: TimelineTarget(roomId: roomId, threadRoot: nil))
        } else {
            navigation.chat.receiveDraft(text)
            navigation.pages.open(.chat)
        }
    }

    private func open(_ entry: PluginEntry) {
        navigation.request { nav in
            switch entry {
            case let .web(tab): try await nav.open(.tab(tab: tab))
            case let .chat(botUserId):
                try await nav.open(.tab(tab: .chat))
                nav.chat.openDirectChat(with: botUserId)
            }
        }
    }
}

private struct ContentRoom: View {
    @Bindable var model: ChatModel
    let plugins: PluginsModel
    let roomId: String

    var body: some View {
        NavigationStack(path: $model.path) {
            ChatRoomView(model: model, plugins: plugins, roomId: roomId, title: model.title(of: roomId))
                .environment(\.wontonContentRoot, true)
                .navigationDestination(for: ChatRoute.self) { route in
                    ChatDestination(model: model, plugins: plugins, route: route)
                        .environment(\.wontonContentRoot, false)
                }
        }
    }
}

private struct ContentWebPage: View {
    let pages: PagesModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let openCardRoom: (String) -> Void
    let closed: () -> Void
    @State private var model: ContentPageModel

    init(core: ClientCore, tab: TabRef, pages: PagesModel, home: HomeModel, plugins: PluginsModel,
         chat: ChatModel, appearance: AppearanceModel, draft: @escaping (String) -> Void,
         open: @escaping (PluginEntry) -> Void, openCardRoom: @escaping (String) -> Void,
         closed: @escaping () -> Void) {
        self.pages = pages
        self.home = home
        self.plugins = plugins
        self.chat = chat
        self.appearance = appearance
        self.draft = draft
        self.open = open
        self.openCardRoom = openCardRoom
        self.closed = closed
        _model = State(initialValue: ContentPageModel(core: core, tab: tab))
    }

    var body: some View {
        Group {
            if let page = model.page {
                TabPageView(
                    pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
                    tab: model.tab, placed: .content(tab: model.tab), store: model.webViews, page: page,
                    draft: draft, open: open, pendingCards: chat.viewModel.pendingConfirmations,
                    openCardRoom: openCardRoom, close: { model.close(); closed() }
                )
            } else if let problem = model.problem {
                ProblemView(problem: problem) { model.attempt += 1 }
            } else {
                ProgressView()
            }
        }
        .task(id: model.attempt) { await model.observe() }
        .onDisappear { model.close() }
    }
}
