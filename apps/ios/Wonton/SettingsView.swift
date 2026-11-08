import SwiftUI
import WontonClient

struct SettingsTab: View {
    @Bindable var model: SettingsModel
    let appearance: AppearanceModel
    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    var isModal = false
    var isActive = true
    @Environment(\.dismiss) private var dismiss
    @State private var isEditingProfile = false
    @State private var isChoosingAgent = false
    @Environment(OnboardingModel.self) private var onboarding

    var body: some View {
        NavigationStack(path: $model.path) {
            SettingsView(model: model, editProfile: { isEditingProfile = true },
                         chooseAgent: { isChoosingAgent = true })
                .wontonRootScreen(TabText.settings, hasActions: isModal) {
                    if isModal { WontonIconButton(icon: .close, label: TabText.close) { dismiss() } }
                }
                .navigationDestination(for: SettingsRoute.self, destination: destination)
        }
        .fullScreenCover(isPresented: $isEditingProfile) { OwnerProfileView(model: plugins) }
        .fullScreenCover(isPresented: $isChoosingAgent) { PrimaryAgentPicker(model: onboarding) }
        .wontonDialog(problem: home.rejection, isPresented: Binding(
            get: { isActive && home.rejection != nil }, set: { if !$0 && isActive { home.rejection = nil } }
        ))
        .wontonDialog(problem: plugins.rejection, isPresented: Binding(
            get: { isActive && plugins.rejection != nil }, set: { if !$0 && isActive { plugins.rejection = nil } }
        ))
    }

    @ViewBuilder private func destination(_ route: SettingsRoute) -> some View {
        switch route {
        case .server: ConnectionView(model: home, plugins: plugins)
        case .connectionDetails: ConnectionDetailsView(model: home)
        case .invite: InviteView(model: home, chat: chat)
        case .pairing:
            PairingView(model: home) { if !model.path.isEmpty { model.path.removeLast() } }
                .wontonPushedScreen(String(localized: .pairingTitleAgain))
        case .peers:
            PeersView(core: chat.core, focusRequests: model.peersFocusRequests)
        case .egress: EgressSettingsView(model: plugins)
        case .registryMirror: RegistryMirrorSettingsView(model: plugins)
        case .coreUpgrade: CoreUpgradeView(model: plugins)
        case .appearance: AppearanceView(model: appearance).wontonPushedScreen(String(localized: .appearanceTitle))
        case .chatBackup: ChatBackupView(model: chat)
        case .bottomBar: TabSettingsView(plugins: plugins).wontonPushedScreen(TabText.bottomBar)
        case .notifications: NotificationSettingsView()
        case .about: AboutView()
        }
    }
}

struct SettingsView: View {
    let model: SettingsModel
    let editProfile: () -> Void
    let chooseAgent: () -> Void
    @Environment(OnboardingModel.self) private var onboarding

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = onboarding.rejection ?? onboarding.viewModel.problem?.problem {
                    ProblemRow(problem: problem) { onboarding.send(.refreshOnboarding) }
                }
                ForEach(Array(model.viewModel.sections.enumerated()), id: \.offset) { _, section in
                    WontonGroup {
                        ForEach(Array(section.entries.enumerated()), id: \.offset) { index, entry in
                            if index > 0 { WontonSeparator() }
                            row(entry)
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    @ViewBuilder private func row(_ entry: SettingsEntry) -> some View {
        switch entry {
        case .profile:
            Button(action: editProfile) {
                WontonRow(title: String(localized: .onboardingStepProfile), showsForward: true)
            }
        case let .server(name, status, upgrade):
            NavigationLink(value: SettingsRoute.server) {
                WontonRow(
                    title: name?.localizedDisplayName ?? String(localized: .serversDefaultFirst),
                    value: status.connectionText, badge: upgrade, showsForward: true
                )
            }
        case .appearance:
            NavigationLink(value: SettingsRoute.appearance) {
                WontonRow(title: String(localized: .appearanceTitle), showsForward: true)
            }
        case .language:
            LanguageSettingsRow()
        case let .peers(count):
            NavigationLink(value: SettingsRoute.peers) {
                WontonRow(title: String(localized: .peersTitle), value: String(count), showsForward: true)
            }
        case .bottomBar:
            NavigationLink(value: SettingsRoute.bottomBar) { WontonRow(title: TabText.bottomBar, showsForward: true) }
        case .notifications:
            NavigationLink(value: SettingsRoute.notifications) {
                WontonRow(title: String(localized: .notificationTitle), showsForward: true)
            }
        case .defaultAgent:
            Button(action: chooseAgent) {
                WontonRow(title: String(localized: .launcherDefaultAgent), showsForward: true)
            }
        case let .chatBackup(recovery):
            NavigationLink(value: SettingsRoute.chatBackup) {
                WontonRow(
                    title: String(localized: .chatBackupTitle), value: recovery.backupValue, showsForward: true
                )
            }
        case .restartOnboarding:
            Button { onboarding.send(.restartOnboarding) } label: {
                WontonRow(title: String(localized: .settingsRestartOnboarding))
            }
        case .about:
            NavigationLink(value: SettingsRoute.about) {
                WontonRow(title: String(localized: .settingsAbout), value: AppVersion.full, showsForward: true)
            }
        }
    }
}

private enum AppVersion {
    static var short: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? String(localized: .commonUnknown)
    }
    static var full: String {
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
            ?? String(localized: .commonUnknown)
        return "\(short) (\(build))"
    }
}

private struct AboutView: View {
    var body: some View {
        ScrollView {
            WontonGroup { WontonRow(title: String(localized: .pluginInfoVersion), value: AppVersion.full) }
                .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .settingsAbout))
    }
}
