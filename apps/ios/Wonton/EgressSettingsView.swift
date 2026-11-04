import SwiftUI
import WontonClient

/// 出网代理的设置页（ADR-0023）：结论都在核心，这里渲染，并把输入框的内容发回去。
struct EgressSettingsView: View {
    let model: PluginsModel
    @State private var address = ""
    @State private var directCidrs = ""
    @Environment(\.wontonColors) private var colors

    private var egress: EgressViewModel { model.viewModel.egress }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen("出网代理") {
                if egress.saving {
                    ProgressView().padding(.trailing, WontonTheme.spacing.s12)
                } else {
                    Button("保存") { save(address: address) }
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(egress.current == nil)
                        .padding(.trailing, WontonTheme.spacing.s12)
                }
            }
            .onAppear { model.send(.refreshEgressProxy) }
            // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
            .onChange(of: egress.current?.address, initial: true) { _, value in address = value ?? "" }
            .onChange(of: egress.current?.directCidrs, initial: true) { _, value in directCidrs = value ?? "" }
            .wontonDialog(
                egress.confirmation?.cancelling == true ? "取消代理要重启这些应用" : "配上代理要重启这些应用",
                message: egress.confirmation.map { $0.restarts.map(\.title).joined(separator: "、") + "\n数据不动。" },
                // 关掉弹窗由按钮发给核心，这里不用再发一次。
                isPresented: .constant(egress.confirmation != nil),
                actions: [
                    WontonDialogAction(title: "取消") { model.send(.dismissEgressConfirmation) },
                    WontonDialogAction(title: "重启", kind: .primary) { model.send(.confirmEgressProxy) }
                ],
                onDismiss: { model.send(.dismissEgressConfirmation) }
            )
    }

    @ViewBuilder private var content: some View {
        if let current = egress.current {
            form(current)
        } else if let problem = egress.loadProblem {
            ProblemView(problem: problem.problem) { model.send(.refreshEgressProxy) }
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
                        EgressPathRow(title: "应用", state: plugin)
                        WontonSeparator()
                        EgressPathRow(title: "家里的服务器", state: core)
                    }
                }
                rangesSection(current)
                if current.settings.url != nil, !current.addressLocked {
                    Button(role: .destructive) {
                        save(address: "")
                    } label: {
                        Text("取消代理").frame(maxWidth: .infinity)
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
        DetailSection(title: "代理地址") {
            // verbatim：字符串字面量会按 Markdown 认成链接，占位符显示成蓝色，像是已经填好了。
            TextField("代理地址", text: $address, prompt: Text(verbatim: "http://192.168.1.2:7890"))
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .disabled(current.addressLocked)
                .wontonField()
            switch current.settings.source {
            case .env:
                secondary("地址由 WONTON_EGRESS_PROXY 指定。要改的话，改安装目录的设置，再运行一次安装脚本。")
            case .detected:
                secondary("安装时检测到的。")
            case .unset, .settings, .unknown:
                EmptyView()
            }
        }
    }

    private func rangesSection(_ current: EgressCurrent) -> some View {
        DetailSection(title: "直连网段") {
            TextField("直连网段", text: $directCidrs, prompt: Text(verbatim: "100.64.0.0/10"))
                .keyboardType(.numbersAndPunctuation)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .wontonField()
            if !current.settings.hostCidrs.isEmpty {
                secondary("主机所在网段：" + current.settings.hostCidrs.joined(separator: ", "))
            }
            secondary("这些网段不经代理，出网应用都能连。")
            ForEach(current.settings.notices, id: \.self) { secondary($0, tint: colors.warning) }
        }
    }

    private func secondary(_ text: String, tint: Rgb? = nil) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(tint ?? colors.textSecondary))
    }

    private func save(address: String) {
        model.send(.saveEgressProxy(address: address, directCidrs: directCidrs))
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
        case .probing: return "探测中"
        case .reachable: return "通"
        case .uncertain: return "不确定"
        case .failed: return "不通"
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
                    Text("在局域网开放网页")
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text("端口 " + offer.ports.map { String($0.port) }.joined(separator: ", "))
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
