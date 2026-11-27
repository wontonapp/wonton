import SwiftUI
import WontonClient

struct SettingsTab: View {
    @Bindable var model: SettingsModel
    let appearance: AppearanceModel
    let appLock: AppLockModel
    let chat: ChatModel
    let home: HomeModel
    let plugins: PluginsModel
    let open: (PluginEntry) -> Void
    let checkInstalled: () -> Void
    let push: PushModel
    var isModal = false
    var isActive = true
    @Environment(\.dismiss) private var dismiss
    @State private var isEditingProfile = false
    @State private var licenses = LicensesModel()
    @Environment(OnboardingModel.self) private var onboarding

    var body: some View {
        NavigationStack(path: $model.path) {
            settingsRoot
        }
        .fullScreenCover(isPresented: $isEditingProfile) { OwnerProfileView(model: plugins) }
        .task { try? chat.core.dispatch(action: .friends(action: .refreshFriends)) }
        .wontonDialog(problem: home.rejection, isPresented: Binding(
            get: { isActive && home.rejection != nil }, set: { if !$0 && isActive { home.rejection = nil } }
        ))
        .wontonDialog(problem: plugins.rejection, isPresented: Binding(
            get: { isActive && plugins.rejection != nil }, set: { if !$0 && isActive { plugins.rejection = nil } }
        ))
    }

    @ViewBuilder
    private var settingsRoot: some View {
        if isModal {
            settingsContent
                .wontonModalScreen(TabText.settings, usesSystemToolbar: true)
        } else {
            settingsContent
                .wontonRootScreen(TabText.settings, hasActions: false) { EmptyView() }
        }
    }

    private var settingsContent: some View {
        SettingsView(model: model, editProfile: { isEditingProfile = true })
        .navigationDestination(for: SettingsRoute.self, destination: destination)
    }

    @ViewBuilder private func destination(_ route: SettingsRoute) -> some View {
        switch route {
        case .server: ConnectionView(model: home, plugins: plugins)
        case .matrixLogin:
            MatrixLoginView(model: home) {
                if model.path.last == .matrixLogin { model.path.removeLast() }
            }
        case .connectionDetails: ConnectionDetailsView(model: home, plugins: plugins)
        case .invite: InviteView(model: home, chat: chat)
        case .pairing:
            PairingView(
                model: home,
                onBack: { if !model.path.isEmpty { model.path.removeLast() } },
                onPaired: { if !model.path.isEmpty { model.path.removeLast() } }
            )
                .wontonPushedScreen(String(localized: .Pairing.pairingTitleAgain))
        case .peers:
            PeersView(core: chat.core, focusRequests: model.peersFocusRequests)
        case .friends: FriendsView(core: chat.core)
        case .egress: EgressSettingsView(model: plugins)
        case .registryMirror: RegistryMirrorSettingsView(model: plugins)
        case .coreUpgrade: CoreUpgradeView(model: plugins)
        case .appearance:
            AppearanceView(model: appearance)
                .wontonPushedScreen(String(localized: .Appearance0.appearanceTitle), usesSystemToolbar: true)
        case .chatBackup: ChatBackupView(model: chat)
        case .myApps:
            MyAppsSettingsView(chat: chat, open: open, checkInstalled: checkInstalled)
                .wontonPushedScreen(TabText.myApps, usesSystemToolbar: true)
        case .notifications: NotificationSettingsView(settings: model, push: push)
        case .appLock: AppLockSettingsView(model: appLock).wontonPushedScreen(String(localized: .Settings.appLockTitle))
        case .about: AboutView(licenses: licenses, core: chat.core)
        case .privacy: PrivacyView(core: chat.core)
        case .licenses: LicensesView(model: licenses, core: chat.core)
        case let .licenseText(id):
            if let license = licenses.license(id: id) {
                LicenseTextView(license: license)
            } else {
                // 列表被重载或换成失败态后，路由里的 id 可能已经取不到：留一句问题话，别白页。
                Text(.Settings.settingsLicensesProblem)
                    .wontonText(WontonTheme.typography.body)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .padding(WontonTheme.spacing.pageInset)
                    .wontonPushedScreen(String(localized: .Settings.settingsLicenses), usesSystemToolbar: true)
            }
        }
    }
}

struct SettingsView: View {
    let model: SettingsModel
    let editProfile: () -> Void
    @Environment(OnboardingModel.self) private var onboarding
    @Environment(\.appVersionOverride) private var appVersionOverride

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = onboarding.rejection ?? onboarding.viewModel.problem?.problem {
                    // 核心在重新取（主人点的，或连着时自己再取）时“重试”转圈、不能再点；有了结果核心换掉提示。
                    ProblemRow(problem: problem, isRetrying: onboarding.viewModel.loading) {
                        onboarding.send(.onboarding(action: .refreshOnboarding))
                    }
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
                WontonRow(title: String(localized: .Onboarding4.onboardingStepProfile), showsForward: true)
            }
        case let .server(name, status, upgrade):
            NavigationLink(value: SettingsRoute.server) {
                WontonRow(
                    title: name?.localizedDisplayName ?? String(localized: .Servers.serversDefaultFirst),
                    value: status.connectionText, badge: upgrade, showsForward: true
                )
            }
        case .appearance:
            NavigationLink(value: SettingsRoute.appearance) {
                WontonRow(title: String(localized: .Appearance0.appearanceTitle), showsForward: true)
            }
        case .language:
            LanguageSettingsRow()
        case let .peers(count):
            NavigationLink(value: SettingsRoute.peers) {
                WontonRow(title: String(localized: .Peers3.peersTitle), value: String(count), showsForward: true)
            }
        case let .friends(count, requests):
            NavigationLink(value: SettingsRoute.friends) {
                WontonRow(
                    title: String(localized: .Friends2.friendsTitle),
                    value: String(count),
                    badgeText: requests > 0 ? String(requests) : nil,
                    showsForward: true
                )
            }
        case .myApps:
            NavigationLink(value: SettingsRoute.myApps) { WontonRow(title: TabText.myApps, showsForward: true) }
        case let .notifications(_, pushStatus, _):
            NavigationLink(value: SettingsRoute.notifications) {
                WontonRow(
                    title: String(localized: .Settings.notificationTitle), value: value(pushStatus), showsForward: true
                )
            }
        case let .appLock(enabled, timeout):
            NavigationLink(value: SettingsRoute.appLock) {
                WontonRow(
                    title: String(localized: .Settings.appLockTitle),
                    value: enabled ? timeout.label : nil,
                    showsForward: true
                )
            }
        case let .chatBackup(recovery):
            NavigationLink(value: SettingsRoute.chatBackup) {
                WontonRow(
                    title: String(localized: .ChatBackup1.chatBackupTitle), value: recovery.backupValue,
                        showsForward: true
                )
            }
        case .restartOnboarding:
            Button { onboarding.send(.onboarding(action: .restartOnboarding)) } label: {
                WontonRow(title: String(localized: .Settings.settingsRestartOnboarding))
            }
        case .about:
            NavigationLink(value: SettingsRoute.about) {
                WontonRow(
                    title: String(localized: .Settings.settingsAbout),
                    value: appVersionOverride ?? AppVersion.full,
                    showsForward: true
                )
            }
        // 成员页由成员那一张卡补上，入口先不画。
        case .members, .matrixAccount, .matrixDevices, .addWontonServer, .other:
            EmptyView()
        }
    }

    /// 设置列表里“通知”条目的值，短的；设置页里是整句话。
    private func value(_ status: PushStatus) -> String {
        switch status {
        case .registered: String(localized: .Settings.pushValueOn)
        case .registering: String(localized: .Settings.pushValueRegistering)
        case .noEndpoint: String(localized: .Settings.pushValueNotSet)
        case .unavailable, .failed: String(localized: .Settings.pushValueUnavailable)
        }
    }
}

enum AppVersion {
    static var short: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? String(localized: .Common.commonUnknown)
    }
    static var full: String {
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
            ?? String(localized: .Common.commonUnknown)
        return "\(short) (\(build))"
    }
}

/// 截图测试钉版本号用：设置页直接读 Bundle 的版本，CI 的 Debug 构建没有 tag 时会变，参考图跟着漂。
/// 测试夹具设一个固定值；生产不设，照常读真实版本。
struct AppVersionOverrideKey: EnvironmentKey {
    static let defaultValue: String? = nil
}

extension EnvironmentValues {
    var appVersionOverride: String? {
        get { self[AppVersionOverrideKey.self] }
        set { self[AppVersionOverrideKey.self] = newValue }
    }
}
