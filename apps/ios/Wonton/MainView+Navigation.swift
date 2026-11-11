import SwiftUI
import WontonClient

extension MainView {
    func receiveAppURL(_ url: URL) {
        if home.prefillPairing(from: url) {
            showsSettings = false
            showsTabSettings = false
            tabs.send(.closeTabOverlay)
            tabs.select(.apps)
            return
        }
        navigation.receive(url)
    }

    func applyAppNavigation() {
        guard navigation.presentationRevision > 0, navigation.allowsNavigation else { return }
        let dismissingCover = overlayPresented.wrappedValue || onboardingGateway != nil || showsTabSettings
        showsSettings = false
        showsTabSettings = false
        onboardingGateway = nil
        pendingSettingsPresentation = navigation.opensSettings && dismissingCover
        if navigation.opensSettings {
            settings.path = []
            tabs.send(.closeTabOverlay)
            if !dismissingCover { showsSettings = true }
        }
        if navigation.takeOpensPeers() {
            settings.path = [.peers]
            settings.focusPeersRequests()
        }
    }

    func showServerRoute(_ route: SettingsRoute) {
        let dismissingCover = overlayPresented.wrappedValue || onboardingGateway != nil || showsTabSettings
        showsTabSettings = false
        onboardingGateway = nil
        settings.openServer(route)
        if tabs.isPinned(.settings) {
            pendingSettingsPresentation = false
            showsSettings = false
            tabs.select(.settings)
        } else {
            pendingSettingsPresentation = dismissingCover
            tabs.send(.closeTabOverlay)
            if !dismissingCover { showsSettings = true }
        }
    }

    func presentPendingSettings() {
        guard pendingSettingsPresentation else { return }
        pendingSettingsPresentation = false
        showsSettings = true
    }

    func openCardRoom(_ roomId: String) {
        showsSettings = false
        showsTabSettings = false
        onboardingGateway = nil
        tabs.select(.chat)
        chat.visiting = nil
        chat.path = [.room(roomId)]
    }

    func finishOnboarding(_ bot: String?) {
        tabs.select(.chat)
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
                title: String(localized: .onboardingGatewayMissingTitle),
                nextStep: String(localized: .onboardingGatewayMissingNextStep)
            )
            return
        }
        onboardingGateway = choice
        tabs.select(choice.tab)
    }

    func receiveDraft(_ text: String) {
        chat.receiveDraft(text)
        tabs.select(.chat)
    }

    func open(_ entry: PluginEntry) {
        switch entry {
        case let .chat(botUserId):
            let origin: AppTab = if tabs.viewModel.overlay != nil {
                .chat
            } else if tabs.viewModel.selected.isWebPage {
                .chat
            } else { nativeTab }
            if chat.openDirectChat(with: botUserId, from: origin) { tabs.select(.chat) }
        case let .web(tab): tabs.select(tab)
        }
    }
}
