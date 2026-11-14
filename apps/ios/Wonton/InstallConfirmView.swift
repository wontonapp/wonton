import SwiftUI
import WontonClient

/// 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。
struct InstallConfirmView: View {
    let model: PluginsModel
    let confirmation: PluginConfirmation
    /// 核心取好预览的确认页（HomeView 那层）：确认后核心自己把这条撤走、跟着关页或放下一条，
    /// 这里不再主动 dismiss——和核心撤走抢先后会在“全部更新”排队时多发一次撤回、跳过下一条。
    var coreOffered = false
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: Result<InstallPreview, Error>?
    @State private var rejection: Problem?
    @State private var lanWebui = false
    @State private var enabled: Set<String> = []
    @State private var devices: [PluginDevice] = []
    @State private var audience: PluginAudienceView?
    @State private var secrets: [String: String] = [:]
    @State private var isFillingProfile = false
    @State private var submittedId: String?
    @State private var hostAcknowledgement = false

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if let submittedId {
                    InstallCompletionView(model: model, pluginId: submittedId, close: { dismiss() }, review: {
                        self.submittedId = nil
                        Task { await load() }
                    })
                } else { content }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if submittedId == nil, let preview {
                Button {
                    requestConfirmation(preview)
                } label: {
                    Text(actionTitle).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(!canConfirm)
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .wontonModalScreen(confirmation.actionTitle, showsStatus: true)
        // 这一屏有密钥输入，切后台的快照里整页盖住。
        .secretSnapshotMask()
        .wontonDialog(problem: rejection, isPresented: isShowingRejection)
        .wontonDialog(
            String(localized: .Plugin3.pluginPermissionUnderstandTitle),
            message: String(localized: .Plugin0.pluginPermissionHostRoot),
            isPresented: $hostAcknowledgement,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)),
                WontonDialogAction(title: actionTitle, kind: .danger) {
                    if let preview { confirm(preview) }
                }
            ]
        )
        .task { await load() }
        // 填完回到这里，预览重新算，缺的资料有了就能装。
        .fullScreenCover(
            isPresented: $isFillingProfile,
            onDismiss: { Task { await load() } },
            content: { OwnerProfileView(model: model) }
        )
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            LoadingView()
        case let .failure(error):
            ProblemView(problem: error.problem) {
                Task { await load() }
            }
        case let .success(preview):
            PreviewContent(model: model, preview: preview, lanWebui: $lanWebui, enabled: $enabled,
                           devices: $devices, audience: $audience, secrets: $secrets) {
                isFillingProfile = true
            }
        }
    }

    private var actionTitle: String {
        if let preview, previewInstallAction(preview: preview) == .requestAdmin {
            return String(localized: .Install.installRequestAdmin)
        }
        return confirmation.actionTitle
    }

    private var isUpgrade: Bool { confirmation.isUpgrade }

    private var preview: InstallPreview? {
        if case let .success(preview) = loaded { return preview }
        return nil
    }

    /// 装不了（`blocked`）、缺资料、有核心不支持的权限、缺必填密钥或必选的串口没选时不能确认。
    private var canConfirm: Bool {
        guard let preview else { return false }
        return previewInstallAction(preview: preview) != .blocked && preview.missingProfile == nil
            && preview.permissions.allSatisfy(\.supported)
            && !missingRequiredSecret(preview.secretInputs, secrets)
            && deviceChoices(devices: devices, permissions: preview.permissionLines(enabling: enabled))?.ready != false
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { rejection != nil },
            set: { isShowing in
                if !isShowing { rejection = nil }
            }
        )
    }

    private func load() async {
        if let preloaded = confirmation.preloaded {
            lanWebui = preloaded.lanWebui?.enabled ?? false
            devices = preloaded.devices ?? []
            enabled = Set(preloaded.permissions.filter { $0.optional && $0.enabled }.map(\.permission))
            audience = preloaded.audience
            loaded = .success(preloaded)
            return
        }
        loaded = nil
        do {
            let preview = switch confirmation {
            case let .install(source): try await model.core.previewInstall(source: source)
            case let .upgrade(id): try await model.core.previewUpgrade(id: id)
            case let .upload(archive): try await model.core.previewUpload(archive: archive)
            case let .upgradeFrom(id, source): try await model.core.previewUpgradeFrom(id: id, source: source)
            case let .upgradeUpload(id, archive): try await model.core.previewUpgradeUpload(id: id, archive: archive)
            case .store: throw CancellationError()
            }
            lanWebui = preview.lanWebui?.enabled ?? false
            devices = preview.devices ?? []
            enabled = Set(preview.permissions.filter { $0.optional && $0.enabled }.map(\.permission))
            audience = preview.audience
            loaded = .success(preview)
        } catch {
            loaded = .failure(error)
        }
    }

    private func requestConfirmation(_ preview: InstallPreview) {
        let needsHost = preview.permissions.contains { line in
            line.requiresHostConfirmation && (!line.optional || enabled.contains(line.permission))
                && (line.optional || line.risk != nil)
        }
        if needsHost {
            hostAcknowledgement = true
        } else {
            confirm(preview)
        }
    }

    /// 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
    private func confirm(_ shown: InstallPreview) {
        var preview = shown
        preview.lanWebui?.enabled = lanWebui
        preview.audience = audience
        preview.permissions = preview.permissionLines(enabling: enabled)
        if preview.devices != nil { preview.devices = devices }
        let secrets = secretValues(preview.secretInputs, self.secrets)
        let action: Action
        if isUpgrade {
            action = .plugins(action: .upgradePlugin(preview: preview, secrets: secrets, settings: [:]))
        } else {
            action = .plugins(action: .installPlugin(preview: preview, secrets: secrets, settings: [:]))
        }
        do {
            try model.core.dispatch(action: action)
            if isUpgrade {
                if !coreOffered { dismiss() }
            } else {
                submittedId = preview.id
            }
        } catch {
            rejection = error.problem
        }
    }
}

private struct PreviewContent: View {
    let model: PluginsModel
    let preview: InstallPreview
    @Binding var lanWebui: Bool
    @Binding var enabled: Set<String>
    @Binding var devices: [PluginDevice]
    @Binding var audience: PluginAudienceView?
    @Binding var secrets: [String: String]
    let fillProfile: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                header
                if let blocked = preview.blocked {
                    WontonNotice(icon: .blocked, title: blocked)
                }
                if let missing = preview.missingProfile {
                    WontonNotice(icon: .info, title: missing, isError: false) {
                        Button(String(localized: .Install.installFillProfile), action: fillProfile)
                            .buttonStyle(WontonButtonStyle())
                    }
                }
                if !preview.notices.isEmpty { NoticeLines(lines: preview.notices) }
                if let binding = Binding($audience) {
                    PluginAudienceSection(audience: binding)
                }
                if !preview.permissions.isEmpty || !preview.widgetCapabilities.isEmpty {
                    DetailSection(title: String(localized: .Install.installPermissions)) {
                        ForEach(preview.permissions, id: \.permission) { line in
                            if line.optional {
                                PermissionRow(line: line, selection: Binding(
                                    get: { enabled.contains(line.permission) },
                                    set: { isOn in
                                        if isOn {
                                            enabled.insert(line.permission)
                                        } else {
                                            enabled.remove(line.permission)
                                        }
                                    }
                                ))
                            } else {
                                PermissionRow(line: line)
                            }
                        }
                        WidgetCapabilityRows(capabilities: preview.widgetCapabilities)
                    }
                }
                if let choices = deviceChoices(
                    devices: devices, permissions: preview.permissionLines(enabling: enabled)
                ) {
                    DeviceChoicesSection(choices: choices) { id in
                        devices = toggleDevice(devices: devices, id: id)
                    }
                }
                if !preview.secretInputs.isEmpty {
                    DetailSection(title: String(localized: .Plugins.secretSectionTitle)) {
                        SecretInputsSection(inputs: preview.secretInputs, values: secrets) { name, value in
                            secrets[name] = value
                        }
                    }
                }
                AlwaysAllowedSection(model: model, preview: preview)
                if let offer = preview.lanWebui {
                    LanWebuiToggle(offer: offer, isOn: $lanWebui)
                }
                if let notes = preview.releaseNotes {
                    DetailSection(title: String(localized: .Install.installReleaseNotes)) {
                        Text(notes)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    private var header: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: preview.icon, size: WontonTheme.size.previewIcon, model: model)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(preview.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Text(version)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                HStack(spacing: WontonTheme.spacing.s8) {
                    Tag(text: preview.trust.label, tint: preview.trust.tint(colors))
                    // 提供方是核实过的，有它就不写清单里自称的作者；两个都放一行放不下。
                    if let origin = preview.provider ?? preview.author?.name {
                        Text(origin)
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
            }
        }
    }

    private var version: String {
        guard let from = preview.upgradeFrom else { return preview.version }
        return "\(from) → \(preview.version)"
    }
}

/// 升级时已经始终允许的 Bot，当场可以关。关掉是立刻生效的，主人不确认升级也一样。
private struct AlwaysAllowedSection: View {
    let model: PluginsModel
    let preview: InstallPreview
    @State private var bots: [BotSwitch]
    @State private var problem: Problem?

    init(model: PluginsModel, preview: InstallPreview) {
        self.model = model
        self.preview = preview
        _bots = State(initialValue: preview.alwaysAllowed)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            BotSwitches(
                title: String(localized: .ToolPolicy.toolPolicyAlwaysTitle),
                risk: String(localized: .Install.installAlwaysAllowedRisk),
                bots: bots,
                enabled: true,
                change: change
            )
            if let problem { ProblemNotice(problem: problem) }
        }
    }

    private func change(caller: String, isOn: Bool) {
        Task {
            do {
                _ = try await model.core.setToolAlways(id: preview.id, caller: caller, enabled: isOn)
                problem = nil
                if let index = bots.firstIndex(where: { $0.caller == caller }) {
                    bots[index].enabled = isOn
                }
            } catch {
                problem = error.problem
            }
        }
    }
}
