import SwiftUI
import WontonClient

/// 确认页上缺的必需能力、一起装的提供方和缺的可选能力（ADR-0053）：提供方单独列一节权限、密钥和设备，
/// 高权限的标出来。
struct InstallProviderSections: View {
    let model: PluginsModel
    let preview: InstallPreview
    let update: (InstallPreview) -> Void
    let fillProfile: () -> Void
    @State private var switching: String?
    @State private var problem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if !wanting.isEmpty && connectsOnUpgrade {
            InstallConnectedApps(
                wanting: wanting,
                connected: preview.connectedPlugins,
                direct: isDirect,
                toggle: toggleConnected
            )
        }
        ForEach(preview.installProviders, id: \.capability) { section in
            DetailSection(title: String(localized: .Install.installRequiredCapabilities)) {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        Text(section.capability)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                        if section.thirdParty { thirdPartyTag }
                    }
                    if section.providers.isEmpty {
                        WontonNotice(icon: .blocked, title: String(localized: .Install.installProviderMissing))
                    }
                    providerChoices(section)
                    if let selected = section.selected {
                        provider(selected, capability: section.capability)
                    }
                }
            }
        }
        if !preview.missingWants.isEmpty {
            DetailSection(title: String(localized: .Install.installOptionalCapabilities)) {
                ForEach(preview.missingWants, id: \.capability) { missing in
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        HStack(spacing: WontonTheme.spacing.s8) {
                            Text(missing.capability)
                                .wontonText(WontonTheme.typography.body)
                                .foregroundStyle(Color(colors.textPrimary))
                            if missing.thirdParty { thirdPartyTag }
                        }
                        Text(.Install.installWantsMissing(missing.capability))
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
            }
        }
        if let problem { ProblemNotice(problem: problem) }
    }

    /// 装的是提供方：已装并想用这个能力的插件（ADR-0053 第 3 条）。外层装提供方时勾选的才接上；
    /// 随批安装的提供方内层带不了连接，只列出来。
    private var wanting: [CapabilityProviderRef] {
        if !preview.wantingPlugins.isEmpty {
            return deduped(preview.wantingPlugins)
        }
        return deduped(
            preview.installProviders.flatMap { $0.selected?.preview.wantingPlugins ?? [] }
        )
    }

    /// 多个提供方各自有人想用时按 ID 去重：一节里的应用不能重复出现。
    private func deduped(_ plugins: [CapabilityProviderRef]) -> [CapabilityProviderRef] {
        var seen = Set<String>()
        return plugins.filter { seen.insert($0.id).inserted }
    }

    /// 升级时只有收 `connect_plugins` 的核（`upgradeConnects`）给开关；旧核知道想用它的应用，但收下
    /// 请求也不认，升级页连这一节都不显示。安装页总是给开关，随批装的提供方内层带不了连接，只列出来。
    private var connectsOnUpgrade: Bool { preview.upgradeFrom == nil || preview.upgradeConnects }

    private var isDirect: Bool { !preview.wantingPlugins.isEmpty && connectsOnUpgrade }

    private var thirdPartyTag: some View {
        Tag(text: String(localized: .Install.installProviderThirdParty), tint: colors.textSecondary)
    }

    @ViewBuilder private func providerChoices(_ section: InstallProvider) -> some View {
        if section.providers.count > 1 {
            VStack(spacing: WontonTheme.spacing.s8) {
                ForEach(section.providers, id: \.id) { candidate in
                    StepOptionRow(
                        title: candidate.title,
                        isSelected: section.selected?.provider.id == candidate.id,
                        select: { select(section, candidate) },
                        leading: { EmptyView() },
                        trailing: {
                            if candidate.highPrivilege { highPrivilegeTag }
                            if switching == candidate.id { ProgressView() }
                        }
                    )
                }
            }
        }
    }

    private var highPrivilegeTag: some View {
        Tag(text: String(localized: .Install.installProviderHighPrivilege), tint: colors.warning)
    }

    @ViewBuilder private func provider(_ selected: SelectedProvider, capability: String) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            HStack(spacing: WontonTheme.spacing.s8) {
                Tag(text: selected.preview.trust.label, tint: selected.preview.trust.tint(colors))
                if let origin = selected.preview.provider ?? selected.preview.author?.name {
                    Text(origin)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                Spacer(minLength: 0)
                if selected.provider.highPrivilege { highPrivilegeTag }
            }
            if selected.provider.highPrivilege {
                Text(.Install.installProviderConnects(preview.title, selected.provider.title, capability))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.warning))
            }
            if let blocked = selected.preview.blocked {
                WontonNotice(icon: .blocked, title: blocked)
            }
            if let missing = selected.preview.missingProfile {
                WontonNotice(icon: .info, title: missing, isError: false) {
                    Button(String(localized: .Install.installFillProfile), action: fillProfile)
                        .buttonStyle(WontonButtonStyle())
                }
            }
            if !selected.preview.notices.isEmpty { NoticeLines(lines: selected.preview.notices) }
            if !selected.preview.permissions.isEmpty {
                providerSectionTitle(.Install.installProviderPermissionTitle(selected.provider.title)) {
                    ForEach(selected.preview.permissions, id: \.permission) { line in
                        permission(line, capability: capability)
                    }
                }
            }
            if !selected.preview.secretInputs.isEmpty {
                providerSectionTitle(.Install.installProviderSecretTitle(selected.provider.title)) {
                    SecretInputsSection(
                        inputs: selected.preview.secretInputs,
                        values: selected.secrets
                    ) { name, value in
                        changeSecret(capability, name, value)
                    }
                }
            }
            if let devices = selected.preview.devices, !devices.isEmpty,
               let choices = deviceChoices(devices: devices, permissions: selected.preview.permissions) {
                DeviceChoicesSection(choices: choices) { id in toggleDevice(capability, id) }
            }
            if let offer = selected.preview.lanWebui {
                LanWebuiToggle(offer: offer, isOn: lanBinding(capability, selected))
            }
        }
        .padding(WontonTheme.spacing.s12)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }

    private func providerSectionTitle<Content: View>(
        _ title: LocalizedStringResource,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(title)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
            content()
        }
    }

    private func permission(_ line: PermissionLine, capability: String) -> PermissionRow {
        guard line.optional else { return PermissionRow(line: line) }
        return PermissionRow(
            line: line,
            selection: Binding(
                get: { line.enabled },
                set: { setPermission(capability, line.permission, $0) }
            )
        )
    }

    private func lanBinding(_ capability: String, _ selected: SelectedProvider) -> Binding<Bool> {
        Binding(
            get: { selected.preview.lanWebui?.enabled ?? false },
            set: { setLan(capability, $0) }
        )
    }

    private func select(_ section: InstallProvider, _ candidate: CapabilityProvider) {
        Task {
            switching = candidate.id
            defer { switching = nil }
            do {
                let fresh = try await model.core.previewInstall(source: candidate.source)
                guard fresh.id == candidate.id else {
                    problem = Problem(title: String(localized: .Install.installProviderWrong(candidate.title)))
                    return
                }
                problem = nil
                mutate { preview in
                    guard let index = preview.installProviders.firstIndex(where: {
                        $0.capability == section.capability
                    }) else { return }
                    preview.installProviders[index].selected = SelectedProvider(
                        provider: candidate, preview: fresh, secrets: [:]
                    )
                }
            } catch {
                problem = error.problem
            }
        }
    }

    private func setPermission(_ capability: String, _ permission: String, _ isOn: Bool) {
        mutate { preview in
            guard let index = preview.installProviders.firstIndex(where: {
                $0.capability == capability
            }), let line = preview.installProviders[index].selected?.preview.permissions
                .firstIndex(where: { $0.permission == permission }) else { return }
            preview.installProviders[index].selected?.preview.permissions[line].enabled = isOn
        }
    }

    private func toggleConnected(_ id: String) {
        mutate { preview in
            if preview.connectedPlugins.contains(id) {
                preview.connectedPlugins.removeAll { $0 == id }
            } else {
                preview.connectedPlugins.append(id)
            }
        }
    }

    private func changeSecret(_ capability: String, _ name: String, _ value: String) {
        mutate { preview in
            guard let index = preview.installProviders.firstIndex(where: {
                $0.capability == capability
            }) else { return }
            if value.isEmpty {
                preview.installProviders[index].selected?.secrets.removeValue(forKey: name)
            } else {
                preview.installProviders[index].selected?.secrets[name] = value
            }
        }
    }

    private func toggleDevice(_ capability: String, _ id: String) {
        mutate { preview in
            guard let index = preview.installProviders.firstIndex(where: {
                $0.capability == capability
            }), let devices = preview.installProviders[index].selected?.preview.devices else { return }
            preview.installProviders[index].selected?.preview.devices =
                WontonClient.toggleDevice(devices: devices, id: id)
        }
    }

    private func setLan(_ capability: String, _ isOn: Bool) {
        mutate { preview in
            guard let index = preview.installProviders.firstIndex(where: {
                $0.capability == capability
            }) else { return }
            preview.installProviders[index].selected?.preview.lanWebui?.enabled = isOn
        }
    }

    /// `installProviders` 是值类型，改完把整份预览交回去。
    private func mutate(_ change: (inout InstallPreview) -> Void) {
        var copy = preview
        change(&copy)
        update(copy)
    }
}
