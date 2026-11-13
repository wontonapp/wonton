import SwiftUI
import WontonClient

extension MainView {
    func receiveAppURL(_ url: URL) {
        if home.completeMatrixLogin(from: url) { return }
        if home.prefillPairing(from: url) {
            showsSettings = false
            closeApps()
            return
        }
        navigation.receive(url)
    }

    func applyAppNavigation() {
        guard navigation.presentationRevision > 0, navigation.allowsNavigation else { return }
        showsSettings = false
        if navigation.opensSettings || !pages.viewModel.showsApps { closeApps() }
        onboardingGateway = nil
        if navigation.opensSettings {
            settings.path = []
            showsSettings = true
        }
        if navigation.takeOpensPeers() {
            settings.path = [.peers]
            settings.focusPeersRequests()
        }
    }

    /// 设置是主界面上唯一的整屏盖子，下面的层（网格、插件页）都不是盖子，直接弹；关掉设置回到原来那一层。
    func showServerRoute(_ route: SettingsRoute) {
        onboardingGateway = nil
        settings.openServer(route)
        showsSettings = true
    }

    /// 打开“全部应用”这一层：核心记录网格是否显示，路径承载网格上面的商店和详情页。
    func openApps(above routes: [AppsRoute] = []) {
        pages.open(.apps)
        // 没配对时网格自己就是根页面，路径里只放它上面的。
        appsPath = NavigationPath(isPaired ? [AppsRoute.launcher] + routes : routes)
    }

    func closeApps() {
        if pages.viewModel.showsApps { pages.closeCurrent() }
        appsPath = NavigationPath()
    }

    func openCardRoom(_ roomId: String?) {
        showsSettings = false
        closeApps()
        onboardingGateway = nil
        pages.open(.chat)
        chat.resetNavigation()
        if let roomId { chat.path = [.room(roomId)] } else { chat.openConfirmations() }
    }

    func finishOnboarding(_ bot: String?) {
        pages.open(.chat)
        chat.path = []
        chat.visiting = nil
        if let bot { chat.openDirectChat(with: bot) }
    }

    func openOnboardingGateway(_ settings: ModelSettingsPage) {
        let gateway = URL(string: settings.webUrl)?.appendingPathComponent("")
        guard let gateway, let plugin = plugins.viewModel.installed?.plugins.first(where: {
            $0.id == settings.pluginId
                && $0.webUrl.flatMap(URL.init(string:))?.appendingPathComponent("") == gateway
        }),
              let choice = plugin.tabChoices.first else {
            onboarding.rejection = Problem(
                title: String(localized: .Onboarding1.onboardingGatewayMissingTitle),
                nextStep: String(localized: .Onboarding3.onboardingGatewayMissingNextStep)
            )
            return
        }
        onboardingGateway = choice
        pages.open(choice.tab)
    }

    func receiveDraft(_ text: String) {
        chat.receiveDraft(text)
        pages.open(.chat)
    }

    /// 从网格打开应用先收起“全部应用”这一层：关掉页面、退出私聊回到的是聊天，不是网格（Android 同）。
    func open(_ entry: PluginEntry) {
        switch entry {
        case let .chat(botUserId):
            if chat.openDirectChat(with: botUserId, from: .chat) {
                closeApps()
                pages.open(.chat)
            }
        case let .web(tab):
            closeApps()
            pages.open(tab)
        case let .browser(url):
            // App 内浏览器是主界面上的 sheet，网格这一层不是盖子，不用先收起；关掉浏览器回到网格。
            navigation.safari = SafariTarget(url: url)
        }
    }
}
