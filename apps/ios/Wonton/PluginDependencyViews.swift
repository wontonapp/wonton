import SwiftUI
import WontonClient

/// 详情页的依赖异常（ADR-0053）：依赖的插件没在运行、wants 的能力没有提供方。文字按类别写，
/// 下一步按 `dependencyNextStep` 给。
struct PluginDependencyHints: View {
    let model: PluginsModel
    let detail: PluginDetail
    let update: (PluginDetail) -> Void
    @State private var problem: Problem?
    @State private var busy: String?

    var body: some View {
        ForEach(Array(detail.hints.enumerated()), id: \.offset) { _, hint in
            if let step = dependencyNextStep(hint: hint) {
                row(hint: hint, step: step)
            }
        }
        if let problem { ProblemNotice(problem: problem) }
    }

    @ViewBuilder private func row(hint: PluginHint, step: DependencyNextStep) -> some View {
        switch step {
        case .startProvider:
            WontonNotice(icon: .warning, title: stoppedText(hint), isError: false) {
                if let provider = hint.provider {
                    startButton(provider)
                }
            }
        case .connectProvider:
            WontonNotice(icon: .info, title: notConnectedText(hint), isError: false) {
                if let provider = hint.provider, let capability = hint.capability {
                    Button(String(localized: .Plugin2.pluginDependencyConnect)) {
                        connect(provider, capability)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .disabled(busy != nil)
                }
            }
        case .openStore:
            WontonNotice(
                icon: .info,
                title: String(localized: .Plugin0.pluginDependencyMissing(hint.capability ?? "")),
                isError: false,
                lines: [String(localized: .Plugin3.pluginDependencyFindInStore)]
            )
        }
    }

    private func stoppedText(_ hint: PluginHint) -> String {
        String(localized: .Plugin1.pluginDependencyStopped(hint.provider?.title ?? ""))
    }

    private func notConnectedText(_ hint: PluginHint) -> String {
        String(
            localized: .Plugin2.pluginDependencyNotConnected(
                hint.provider?.title ?? "", hint.capability ?? ""
            )
        )
    }

    private func startButton(_ provider: CapabilityProviderRef) -> some View {
        Button {
            busy = provider.id
            model.send(.plugins(action: .startPlugin(pluginId: provider.id)))
            busy = nil
        } label: {
            Text(.Plugin3.pluginDependencyStart(provider.title))
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
    }

    private func connect(_ provider: CapabilityProviderRef, _ capability: String) {
        Task {
            busy = provider.id
            defer { busy = nil }
            do {
                let updated = try await model.core.setPluginCapability(
                    id: detail.id, capability: capability, provider: provider.id
                )
                problem = nil
                update(updated)
            } catch {
                problem = error.problem
            }
        }
    }
}

/// 装着、主人能改时给每种能力选提供方（ADR-0053）：跟全局默认，或者单独指定一个。
struct PluginCapabilityChoices: View {
    let model: PluginsModel
    let detail: PluginDetail
    let update: (PluginDetail) -> Void
    @State private var problem: Problem?
    @State private var busy: String?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Plugin1.pluginCapabilitiesTitle)) {
            ForEach(detail.capabilityProviders, id: \.capability) { setting in
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        Text(setting.capability)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                        if setting.thirdParty {
                            Tag(
                                text: String(localized: .Install.installProviderThirdParty),
                                tint: colors.textSecondary
                            )
                        }
                    }
                    choice(setting, provider: nil, title: String(localized: .Plugin1.pluginCapabilityFollowDefault))
                    ForEach(setting.providers, id: \.id) { provider in
                        choice(setting, provider: provider, title: provider.title)
                    }
                }
            }
            if let problem { ProblemNotice(problem: problem) }
        }
    }

    private func choice(
        _ setting: ConsumerCapabilitySetting,
        provider: CapabilityProviderRef?,
        title: String
    ) -> some View {
        StepOptionRow(
            title: title,
            isSelected: setting.selected == provider?.id,
            select: { choose(setting.capability, provider?.id) },
            leading: { EmptyView() },
            trailing: {
                if provider?.highPrivilege == true {
                    Tag(
                        text: String(localized: .Install.installProviderHighPrivilege),
                        tint: colors.warning
                    )
                }
            }
        )
        .disabled(busy != nil)
    }

    private func choose(_ capability: String, _ provider: String?) {
        Task {
            busy = capability
            defer { busy = nil }
            do {
                let updated = try await model.core.setPluginCapability(
                    id: detail.id, capability: capability, provider: provider
                )
                problem = nil
                update(updated)
            } catch {
                problem = error.problem
            }
        }
    }
}

/// 卸载被在用的依赖挡住（ADR-0053）：列出还在用的插件，给出“一起卸载”或“先停它们”。
struct UninstallDependents: View {
    let model: PluginsModel
    let title: String
    let id: String
    let dependents: [CapabilityProviderRef]

    var body: some View {
        WontonNotice(
            icon: .warning,
            title: String(localized: .Plugin3.pluginUninstallInUse(title)),
            lines: dependents.map(\.title)
        ) {
            Button(String(localized: .Plugin0.pluginUninstallTogether)) {
                model.send(.plugins(action: .uninstallPlugin(
                    id: id,
                    deleteData: false,
                    providers: [],
                    dependents: dependents.map(\.id)
                )))
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            Button(String(localized: .Plugin1.pluginUninstallStopDependents)) {
                for dependent in dependents {
                    model.send(.plugins(action: .stopPlugin(pluginId: dependent.id)))
                }
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
    }
}

extension PluginTask {
    /// 任务失败的类别。卸载被依赖挡住时也是它，任务上带着依赖方。
    var failureReason: PluginFailure? {
        if case let .failed(reason, _) = state { return reason }
        return nil
    }
}

/// 详情页列出缺的必需和可选能力（ADR-0053）：商店详情不打开确认页也要看得到缺什么、有没有提供方。
struct PluginMissingCapabilities: View {
    let requires: [MissingCapability]
    let wants: [MissingCapability]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if !requires.isEmpty || !wants.isEmpty {
            DetailSection(title: String(localized: .Plugin0.pluginDependenciesMissing)) {
                ForEach(requires, id: \.capability) { missing in
                    row(missing, required: true)
                }
                ForEach(wants, id: \.capability) { missing in
                    row(missing, required: false)
                }
            }
        }
    }

    private func row(_ missing: MissingCapability, required: Bool) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s8) {
                let label: LocalizedStringResource = required
                    ? .Plugin0.pluginDependencyRequires(missing.capability)
                    : .Plugin2.pluginDependencyOptional(missing.capability)
                Text(label)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if missing.thirdParty {
                    Tag(text: String(localized: .Install.installProviderThirdParty), tint: colors.textSecondary)
                }
            }
            if missing.providers.isEmpty {
                Text(.Plugin2.pluginDependencyNoProvider)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(required ? colors.warning : colors.textSecondary))
            } else {
                Text(.Plugin3.pluginDependencyProvidedBy(
                    missing.providers.map(\.title).joined(separator: ", ")
                ))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            }
        }
    }
}

/// 想用这个提供方的已装应用（ADR-0053 第 3 条）：直接装提供方时勾选的才接上，随批安装的只列出来。
struct InstallConnectedApps: View {
    let wanting: [CapabilityProviderRef]
    let connected: [String]
    let direct: Bool
    let toggle: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Install.installWantsConnectedTitle)) {
            ForEach(wanting, id: \.id) { plugin in
                HStack(spacing: WontonTheme.spacing.s8) {
                    Text(plugin.title)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    if plugin.highPrivilege {
                        Tag(
                            text: String(localized: .Install.installProviderHighPrivilege),
                            tint: colors.warning
                        )
                    }
                    Spacer(minLength: 0)
                    if direct {
                        Toggle("", isOn: Binding(
                            get: { connected.contains(plugin.id) },
                            set: { _ in toggle(plugin.id) }
                        ))
                        .labelsHidden()
                        .toggleStyle(WontonToggleStyle())
                        .accessibilityLabel(plugin.title)
                    }
                }
            }
        }
    }
}
