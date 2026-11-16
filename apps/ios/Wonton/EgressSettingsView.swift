import SwiftUI
import WontonClient

/// 出网代理的设置页（ADR-0023）：结论都在核心，这里渲染，并把输入框的内容发回去。
struct EgressSettingsView: View {
    let model: PluginsModel
    @State private var address = ""
    @State private var directCidrs = ""
    @State private var usePlugin = false
    @Environment(\.wontonColors) private var colors

    private var egress: EgressViewModel { model.viewModel.egress }
    private var selectedPlugin: Bool { egress.current?.settings.provider != nil && usePlugin }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(String(localized: .Egress.egressTitle), usesSystemToolbar: true) {
                if egress.saving {
                    ProgressView().padding(.trailing, WontonTheme.spacing.s12)
                } else {
                    Button(String(localized: .Common.commonSave)) {
                        save(address: selectedPlugin ? "" : address, usePlugin: selectedPlugin)
                    }
                        .wontonBarAction()
                        .disabled(egress.current == nil)
                        .padding(.trailing, WontonTheme.spacing.s12)
                }
            }
            .onAppear { model.send(.plugins(action: .refreshEgressProxy)) }
            // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
            .onChange(of: egress.current?.address, initial: true) { _, value in address = value ?? "" }
            .onChange(of: egress.current?.directCidrs, initial: true) { _, value in directCidrs = value ?? "" }
            .onChange(of: egress.current?.usePlugin, initial: true) { _, value in usePlugin = value ?? false }
            .wontonDialog(
                String(localized: egress.confirmation?.cancelling == true
                       ? .Egress.egressRestartCancelTitle : .Egress.egressRestartSetTitle),
                message: egress.confirmation.map {
                    String(
                        localized: .Egress.egressRestartMessage(
                            $0.restarts.map(\.title).formatted(.list(type: .and))
                        )
                    )
                },
                // 关掉弹窗由按钮发给核心，这里不用再发一次。
                isPresented: .constant(egress.confirmation != nil),
                actions: [
                    WontonDialogAction(title: String(localized: .Common.commonCancel)) {
                        model.send(.plugins(action: .dismissEgressConfirmation))
                    },
                    WontonDialogAction(title: String(localized: .Egress.egressRestartConfirm), kind: .primary) {
                        model.send(.plugins(action: .confirmEgressProxy))
                    }
                ],
                onDismiss: { model.send(.plugins(action: .dismissEgressConfirmation)) }
            )
    }

    @ViewBuilder private var content: some View {
        if let current = egress.current {
            form(current)
        } else if let problem = egress.loadProblem {
            ProblemView(problem: problem.problem) { model.send(.plugins(action: .refreshEgressProxy)) }
        } else {
            LoadingView()
        }
    }

    private func form(_ current: EgressCurrent) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = egress.saveProblem { ProblemNotice(problem: problem.saveProblem) }
                if let problem = egress.loadProblem { ProblemNotice(problem: problem.problem) }
                addressSection(current)
                if let plugin = current.pluginPath, let core = current.corePath {
                    WontonGroup {
                        EgressPathRow(title: String(localized: .Egress.egressPathApps), state: plugin)
                        WontonSeparator()
                        EgressPathRow(title: String(localized: .Egress.egressPathHomeServer), state: core)
                    }
                }
                rangesSection(current)
                if current.settings.url != nil || current.settings.source == .plugin, !current.addressLocked {
                    Button(role: .destructive) {
                        save(address: "", usePlugin: false)
                    } label: {
                        Text(.Egress.egressTurnOff).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .danger))
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .disabled(egress.saving)
    }

    private func addressSection(_ current: EgressCurrent) -> some View {
        DetailSection(title: String(localized: .Egress.egressAddress)) {
            if let provider = current.settings.provider {
                StepOptionRow(title: String(localized: .Egress.egressAddressUseProvider(provider.title)),
                              isSelected: usePlugin) { usePlugin = true }
                    .disabled(current.addressLocked)
                StepOptionRow(title: String(localized: .Egress.egressAddressEnter),
                              isSelected: !usePlugin) { usePlugin = false }
                    .disabled(current.addressLocked)
            }
            if !usePlugin || current.settings.provider == nil {
                // verbatim：占位地址不是链接，也不是已经填好的内容。
                TextField(String(localized: .Egress.egressAddress), text: $address,
                          prompt: Text(verbatim: "http://192.0.2.10:7890"))
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .disabled(current.addressLocked)
                    .wontonField()
            }
            switch current.settings.source {
            case .env:
                secondary(String(localized: .Egress.egressSourceEnv))
            case .detected:
                secondary(String(localized: .Egress.egressSourceDetected))
            case .plugin:
                if let provider = current.settings.provider {
                    secondary(String(localized: .Egress.egressSourceProvider(provider.title)))
                }
            case .unset, .settings, .unknown:
                EmptyView()
            }
        }
    }

    private func rangesSection(_ current: EgressCurrent) -> some View {
        DetailSection(title: String(localized: .Egress.egressRanges)) {
            TextField(String(localized: .Egress.egressRanges), text: $directCidrs,
                prompt: Text(verbatim: "100.64.0.0/10"))
                .keyboardType(.numbersAndPunctuation)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .wontonField()
            if !current.settings.hostCidrs.isEmpty {
                secondary(String(
                    localized: .Egress.egressRangesHost(
                        current.settings.hostCidrs.joined(separator: ", ")
                    )
                ))
            }
            secondary(String(localized: .Egress.egressRangesNote))
            ForEach(current.settings.notices, id: \.self) { secondary($0, tint: colors.warning) }
        }
    }

    private func secondary(_ text: String, tint: Rgb? = nil) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(tint ?? colors.textSecondary))
    }

    private func save(address: String, usePlugin: Bool) {
        model.send(.plugins(action: .saveEgressProxy(address: address, directCidrs: directCidrs, usePlugin: usePlugin)))
    }
}

/// 一条路径通不通，不通时写原因。
private struct EgressPathRow: View {
    let title: String
    let state: EgressPathState
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            WontonRow(title: title, value: label, valueTint: tint)
            if let message {
                Text(message)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .padding(.horizontal, WontonTheme.spacing.s16)
                    .padding(.bottom, WontonTheme.spacing.s12)
            }
        }
    }

    private var label: String {
        switch state {
        case .probing: return String(localized: .Egress.egressPathProbing)
        case .reachable: return String(localized: .Egress.egressPathReachable)
        case .uncertain: return String(localized: .Egress.egressPathUncertain)
        case .failed: return String(localized: .Egress.egressPathFailed)
        }
    }

    private var tint: Rgb {
        switch state {
        case .probing: return colors.textSecondary
        case .reachable: return colors.success
        case .uncertain: return colors.warning
        case .failed: return colors.danger
        }
    }

    private var message: String? {
        switch state {
        case .probing, .reachable: return nil
        case let .uncertain(message), let .failed(message): return message
        }
    }
}

/// 网页局域网端口的开关（ADR-0023），风险写在旁边。
struct LanWebuiToggle: View {
    let offer: LanWebuiOffer
    @Binding var isOn: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Toggle(isOn: $isOn) {
                VStack(alignment: .leading, spacing: 0) {
                    Text(.Peers.lanWebuiToggle)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    let ports = offer.ports.map { String($0.port) }.joined(separator: ", ")
                    Text(.Peers.lanWebuiPorts(ports: offer.ports.count, ports))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .toggleStyle(WontonToggleStyle())
            Text(offer.risk)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.warning))
        }
    }
}
