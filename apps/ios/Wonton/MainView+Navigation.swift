import SwiftUI
import WontonClient

extension MainView {
    func receiveAppURL(_ url: URL) {
        if home.completeMatrixLogin(from: url) { return }
        if home.prefillPairing(from: url) {
            showsSettings = false
            showsApps = false
            return
        }
        navigation.receive(url)
    }

    func applyAppNavigation() {
        guard navigation.presentationRevision > 0, navigation.allowsNavigation else { return }
        let dismissingCover = pages.showsPluginPage || onboardingGateway != nil || showsApps
        showsSettings = false
        showsApps = false
        onboardingGateway = nil
        pendingSettingsPresentation = navigation.opensSettings && dismissingCover
        if navigation.opensSettings {
            settings.path = []
            if !dismissingCover { showsSettings = true }
        }
        if navigation.takeOpensPeers() {
            settings.path = [.peers]
            settings.focusPeersRequests()
        }
    }

    func showServerRoute(_ route: SettingsRoute) {
        let dismissingCover = pages.showsPluginPage || onboardingGateway != nil || showsApps
        showsApps = false
        onboardingGateway = nil
        settings.openServer(route)
        pendingSettingsPresentation = dismissingCover
        if !dismissingCover { showsSettings = true }
    }

    func presentPendingSettings() {
        guard pendingSettingsPresentation else { return }
        pendingSettingsPresentation = false
        showsSettings = true
    }

    func openCardRoom(_ roomId: String) {
        showsSettings = false
        showsApps = false
        onboardingGateway = nil
        pages.open(.chat)
        chat.visiting = nil
        chat.path = [.room(roomId)]
    }

    func finishOnboarding(_ bot: String?) {
        pages.open(.chat)
        chat.path = []
        chat.visiting = nil
        if let bot { chat.openDirectChat(with: bot) }
    }

    func openOnboardingGateway(_ url: URL) {
        // 两个核心接口的根地址可能分别带和不带末尾斜杠。
        let gateway = url.appendingPathComponent("")
        guard let plugin = plugins.viewModel.installed?.plugins.first(where: {
            $0.webUrl.flatMap(URL.init(string:))?.appendingPathComponent("") == gateway
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

    /// 页面层在“全部应用”的全屏盖子下面：从网格打开先收起盖子，不然页面开在看不见的地方。
    func open(_ entry: PluginEntry) {
        switch entry {
        case let .chat(botUserId):
            if chat.openDirectChat(with: botUserId, from: .chat) {
                showsApps = false
                pages.open(.chat)
            }
        case let .web(tab):
            showsApps = false
            pages.open(tab)
        }
    }
}
