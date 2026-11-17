import Foundation
import Observation
import UIKit
import WontonClient

/// App 和系统 Intent 共用同一套模型；冷启动时也不依赖 MainView 已经出现。
@MainActor
@Observable
final class AppNavigation {
    private(set) static var shared: AppNavigation?

    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    let pages: PagesModel
    let onboarding: OnboardingModel
    let shareIn: ShareInModel
    private(set) var presentationRevision = 0
    private(set) var opensSettings = false
    /// 链接要求直接打开“其他服务器”页并滚到“请求”；由 MainView 取走。
    private var opensPeers = false
    /// 正在用 SFSafariViewController 打开的外部网页。
    var safari: SafariTarget?
    private var pending: Task<Void, Never>?

    init(chat: ChatModel, home: HomeModel, plugins: PluginsModel) {
        self.chat = chat
        self.home = home
        self.plugins = plugins
        pages = PagesModel(core: chat.core)
        onboarding = OnboardingModel(core: chat.core)
        shareIn = ShareInModel(core: chat.core)
        Self.shared = self
    }

    static func requireShared() throws -> AppNavigation {
        guard let shared else {
            throw CoreError.NotReady(reason: String(localized: .Shortcuts.shortcutsNotReadyLaunchFailed))
        }
        return shared
    }

    var allowsNavigation: Bool {
        // 读核心快照，不等 SwiftUI 的订阅任务；冷启动、重新引导都不能绕过门禁。
        chat.core.home().status != .notConfigured && chat.core.onboarding().step == .finished
    }

    func receive(_ url: URL) {
        guard let link = parseAppLink(url: url.absoluteString) else { return }
        // 分享扩展打开 App（ADR-0097）：没配对、引导没走完也认领，分享页自己引导去配对。
        if case let .share(token) = link {
            shareIn.claim(token)
            return
        }
        if case let .tab(tab) = link {
            openTab(tab)
        } else {
            request { try await $0.open(link) }
        }
    }

    /// 打开一个页面引用；外部网页插件不经页面栈，交给系统的 App 内浏览器（ADR-0080）。
    /// 插件列表还没读到时按页面打开：和改造前的行为一致（打开不了如实报）。
    func openTab(_ tab: TabRef) {
        if case let .plugin(pluginId, _) = tab,
           let plugin = plugins.viewModel.installed?.plugins.first(where: { $0.id == pluginId }),
           plugin.webUrl == nil, let url = plugin.linkUrl, let external = URL(string: url) {
            safari = SafariTarget(url: external)
            return
        }
        request { try await $0.open(.tab(tab: tab)) }
    }

    /// 分享页发出去以后打开那个聊天看进度。
    func openSharedRoom(_ roomId: String) {
        request { navigation in
            guard try await navigation.navigationReady() else { return }
            try navigation.select(.chat)
            navigation.chat.resetNavigation()
            navigation.chat.openRoom(id: roomId)
        }
    }

    /// 聊天里的一条链接：类别由核心判断，壳只按类别开（ADR-0065）。外部和认不出的地址用 Safari 在 App 内打开，
    /// 插件进插件页，matrix.to 在 App 里跳用户或房间，邮件和电话交给系统。
    func openLink(_ link: RichLink) {
        switch link.kind {
        case .external, .other:
            guard let url = URL(string: link.url) else { return }
            safari = SafariTarget(url: url)
        case let .plugin(pluginId):
            openTab(.plugin(pluginId: pluginId, page: nil))
        case let .matrix(target):
            switch target {
            case let .user(userId):
                request { navigation in
                    guard try await navigation.navigationReady() else { return }
                    try navigation.select(.chat)
                    navigation.chat.resetNavigation()
                    navigation.chat.openDirectChat(with: userId)
                }
            case let .room(roomId, _):
                request { navigation in
                    guard try await navigation.navigationReady() else { return }
                    try navigation.select(.chat)
                    let roomId = try await navigation.chat.core.resolveChatRoom(input: roomId)
                    navigation.chat.openRoom(id: roomId)
                }
            }
        case .email, .phone:
            guard let url = URL(string: link.url) else { return }
            UIApplication.shared.open(url)
        }
    }

    /// 链接和 Intent 都会把 App 打开，所以交给 App 执行、出错用 App 的对话框说。冷启动时连上家里的服务器可能要十几秒以上，
    /// 请求一直等到引导状态到了，不设期限；新的请求取代还没执行完的旧请求。
    func request(_ work: @escaping @MainActor (AppNavigation) async throws -> Void) {
        pending?.cancel()
        pending = Task { [weak self] in
            guard let self else { return }
            do {
                try await work(self)
            } catch {
                guard !Task.isCancelled, !(error is CancellationError) else { return }
                pages.rejection = (error as? IntentProblem).map { Problem(title: $0.message) } ?? error.problem
            }
        }
    }

    /// 导航在引导期间与 URL 一样忽略；只有发送文字的请求必须明确拒绝。
    func open(_ link: AppLink) async throws {
        guard try await navigationReady() else { return }
        switch link {
        case let .tab(tab):
            if case .plugin = tab {
                try await waitForInitialPlugins()
                guard allowsNavigation else { return }
            }
            try select(tab)
            chat.resetNavigation()
        case .newChat:
            try select(.chat)
            chat.startNewChat()
        case .agentChat:
            let bot = try primaryBot()
            try select(.chat)
            chat.openDirectChat(with: bot, acceptingInvitation: true)
        case let .room(roomId):
            try select(.chat)
            chat.openRoom(id: roomId)
        case .peers:
            opensPeers = true
            opensSettings = true
            presentationRevision += 1
            chat.resetNavigation()
        case let .share(token):
            // `receive` 已经先接走；这里只为别的入口带来的同一个链接。
            shareIn.claim(token)
        case .settings:
            opensSettings = true
            presentationRevision += 1
        }
    }

    /// 链接要求进“其他服务器”。取走标记，调用方去改导航路径。
    func takeOpensPeers() -> Bool {
        defer { opensPeers = false }
        return opensPeers
    }

    func askAgent(_ text: String) async throws {
        guard try await navigationReady() else {
            throw CoreError.NotReady(reason: String(localized: .Shortcuts.shortcutsNotReadyAskAgent))
        }
        let bot = try primaryBot()
        // 新操作不能被上一次后台失败留下的 problem 提前判成失败。
        try chat.core.dispatch(action: .chat(action: .dismissChatProblem))
        try chat.core.dispatch(action: .chat(action: .askAgent(text: text)))
        try select(.chat)
        // AskAgent 已负责建房和接受邀请，不能再发一次 StartDirectChat。
        chat.openAskedAgentChat(with: bot)
    }

    /// 已配对设备冷启动时引导状态先为空；只等恢复，不等用户走完真实的引导步骤。`deadline` 给在后台运行、
    /// 不能一直等的调用（快捷指令编辑器取页面候选）。
    func navigationReady(deadline: Duration? = nil) async throws -> Bool {
        let clock = ContinuousClock()
        let end = deadline.map { clock.now.advanced(by: $0) }
        while chat.core.home().status != .notConfigured {
            try Task.checkCancellation()
            let view = chat.core.onboarding()
            if let step = view.step { return step == .finished }
            if let problem = view.problem { throw IntentProblem(problem: problem.problem) }
            if let end, clock.now >= end {
                throw CoreError.NotReady(reason: String(localized: .Shortcuts.shortcutsNotReadySlowServer))
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        return false
    }

    /// 未固定页面来自首次插件列表，不能把“尚未加载”误当成“已经卸载”。
    func waitForInitialPlugins(deadline: Duration? = nil) async throws {
        let clock = ContinuousClock()
        let end = deadline.map { clock.now.advanced(by: $0) }
        while chat.core.plugins().installed == nil {
            try Task.checkCancellation()
            guard allowsNavigation else { return }
            if let problem = chat.core.plugins().problem { throw IntentProblem(problem: problem.problem) }
            if let end, clock.now >= end {
                throw CoreError.NotReady(reason: String(localized: .Shortcuts.shortcutsNotReadyPluginsNotLoaded))
            }
            try await Task.sleep(for: .milliseconds(100))
        }
    }

    private func primaryBot() throws -> String {
        guard let bot = chat.core.onboarding().primaryBotUserId else {
            throw CoreError.NotReady(reason: String(localized: .Shortcuts.shortcutsNotReadyNoDefaultAgent))
        }
        return bot
    }

    private func select(_ tab: TabRef) throws {
        try pages.openForNavigation(tab)
        opensSettings = false
        presentationRevision += 1
    }
}
