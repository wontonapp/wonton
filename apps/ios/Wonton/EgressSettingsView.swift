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
            .navigationTitle("出网代理")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    if egress.saving {
                        ProgressView()
                    } else {
                        Button("保存") { save(address: address) }.disabled(egress.current == nil)
                    }
                }
            }
            .onAppear { model.send(.refreshEgressProxy) }
            // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
            .onChange(of: egress.current?.address, initial: true) { _, value in address = value ?? "" }
            .onChange(of: egress.current?.directCidrs, initial: true) { _, value in directCidrs = value ?? "" }
            .alert(
                egress.confirmation?.cancelling == true ? "取消代理要重启这些插件" : "配上代理要重启这些插件",
                // 关掉弹窗由按钮发给核心，这里不用再发一次。
                isPresented: .constant(egress.confirmation != nil),
                presenting: egress.confirmation
            ) { _ in
                Button("重启") { model.send(.confirmEgressProxy) }
                Button("取消", role: .cancel) { model.send(.dismissEgressConfirmation) }
            } message: { confirmation in
                Text(confirmation.restarts.map(\.title).joined(separator: "、") + "\n数据不动。")
            }
    }

    @ViewBuilder private var content: some View {
        if let current = egress.current {
            form(current)
        } else if let problem = egress.loadProblem {
            ProblemView(message: problem.message, nextStep: nil) { model.send(.refreshEgressProxy) }
        } else {
            ProgressView()
        }
    }

    private func form(_ current: EgressCurrent) -> some View {
        Form {
            ForEach(problems, id: \.self) { message in
                Text(message).foregroundStyle(Color(colors.danger))
            }
            addressSection(current)
            if let plugin = current.pluginPath, let core = current.corePath {
                Section {
                    EgressPathRow(title: "插件", state: plugin)
                    EgressPathRow(title: "家里的服务器", state: core)
                }
            }
            rangesSection(current)
            if current.settings.url != nil, !current.addressLocked {
                Section {
                    Button("取消代理", role: .destructive) { save(address: "") }
                }
            }
        }
        .disabled(egress.saving)
    }

    private var problems: [String] {
        [egress.saveProblem, egress.loadProblem].compactMap { $0?.message }
    }

    private func addressSection(_ current: EgressCurrent) -> some View {
        Section {
            // verbatim：字符串字面量会按 Markdown 认成链接，占位符显示成蓝色，像是已经填好了。
            TextField("代理地址", text: $address, prompt: Text(verbatim: "http://192.168.1.2:7890"))
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .disabled(current.addressLocked)
        } header: {
            Text("代理地址")
        } footer: {
            switch current.settings.source {
            case .env:
                Text("地址由 WONTON_EGRESS_PROXY 指定。要改的话，改安装目录的设置，再运行一次安装脚本。")
            case .detected:
                Text("安装时检测到的。")
            case .unset, .settings, .unknown:
                EmptyView()
            }
        }
    }

    private func rangesSection(_ current: EgressCurrent) -> some View {
        Section {
            TextField("100.64.0.0/10", text: $directCidrs)
                .keyboardType(.numbersAndPunctuation)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            if !current.settings.hostCidrs.isEmpty {
                LabeledContent("主机所在网段", value: current.settings.hostCidrs.joined(separator: ", "))
            }
            ForEach(current.settings.notices, id: \.self) { notice in
                Text(notice).foregroundStyle(Color(colors.warning))
            }
        } header: {
            Text("直连网段")
        } footer: {
            Text("这些网段不经代理，出网插件都能连。")
        }
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
        VStack(alignment: .leading, spacing: 4) {
            LabeledContent(title) {
                Text(label).foregroundStyle(tint)
            }
            if let message {
                Text(message).font(.footnote).foregroundStyle(.secondary)
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

    private var tint: Color {
        switch state {
        case .probing: return Color(colors.textSecondary)
        case .reachable: return Color(colors.success)
        case .uncertain: return Color(colors.warning)
        case .failed: return Color(colors.danger)
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
        VStack(alignment: .leading, spacing: 6) {
            Toggle(isOn: $isOn) {
                Text("在局域网开放网页")
                Text("端口 " + offer.ports.map { String($0.port) }.joined(separator: ", "))
            }
            Text(offer.risk).font(.footnote).foregroundStyle(Color(colors.warning))
        }
    }
}
