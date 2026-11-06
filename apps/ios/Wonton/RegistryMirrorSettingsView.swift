import SwiftUI
import WontonClient

struct RegistryMirrorSettingsView: View {
    let model: PluginsModel
    @State private var enabled = false
    @State private var selection: String?
    @State private var custom = false
    @State private var customAddress = ""
    @State private var initialized = false
    @State private var savedMirror: String?
    @Environment(\.wontonColors) private var colors

    private var mirror: RegistryMirrorViewModel { model.viewModel.registryMirror }
    private var draft: String { custom ? customAddress : selection ?? "" }
    private var removedChoice: Bool {
        !custom && selection.map { !(mirror.settings?.choices.contains($0) ?? false) } == true
    }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(RegistryMirrorText.title) {
                if mirror.saving {
                    ProgressView().padding(.trailing, WontonTheme.spacing.s12)
                } else {
                    Button(TabText.choose("保存", "Save")) {
                        model.send(.saveRegistryMirror(mirror: enabled ? draft : ""))
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(mirror.settings == nil ||
                              (enabled && draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                    .padding(.trailing, WontonTheme.spacing.s12)
                }
            }
            .onAppear { model.send(.refreshRegistryMirror) }
            .onChange(of: mirror.settings, initial: true) { _, settings in
                synchronize(settings)
            }
    }

    @ViewBuilder private var content: some View {
        if let settings = mirror.settings {
            form(settings)
        } else if let problem = mirror.loadProblem {
            ProblemView(problem: problem.problem) { model.send(.refreshRegistryMirror) }
        } else {
            LoadingView()
        }
    }

    private func form(_ settings: RegistryMirrorSettings) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if settings.recommended && !enabled {
                    Text(RegistryMirrorText.recommended)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.warning))
                }
                if let problem = mirror.loadProblem {
                    ProblemNotice(problem: problem.problem)
                    Button(TabText.retry) { model.send(.refreshRegistryMirror) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        .disabled(mirror.loading)
                }
                if let problem = mirror.saveProblem {
                    ProblemNotice(problem: problem.reason == .invalid
                                  ? Problem(title: TabText.choose("镜像地址没有保存", "Mirror address wasn't saved"),
                                            detail: problem.message)
                                  : problem.saveProblem)
                }
                if let problem = mirror.testProblem { ProblemNotice(problem: problem.problem) }
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    Toggle(RegistryMirrorText.title, isOn: $enabled)
                        .toggleStyle(WontonToggleStyle())
                    Text(RegistryMirrorText.risk)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                if enabled { choices(settings) }
                testButton
                results(settings)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .disabled(mirror.saving)
    }

    private func choices(_ settings: RegistryMirrorSettings) -> some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            let customValue = removedChoice ? selection ?? "" : customAddress
            ForEach(settings.choices, id: \.self) { address in
                StepOptionRow(title: address, isSelected: !custom && selection == address, select: {
                    custom = false
                    selection = address
                }, trailing: {
                    RegistryProbeLabel(probe: probe(address))
                })
            }
            StepOptionRow(title: RegistryMirrorText.custom, isSelected: custom || removedChoice) {
                if removedChoice { customAddress = customValue }
                custom = true
            }
            if custom || removedChoice {
                TextField(RegistryMirrorText.custom, text: Binding(
                    get: { customValue },
                    set: {
                        customAddress = $0
                        custom = true
                    }
                ), prompt: Text(verbatim: "mirror.example.com:5000"))
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                if let result = probe(customValue) {
                    RegistryProbeRow(title: customValue, probe: result)
                }
            }
        }
    }

    private var testButton: some View {
        Button { model.send(.testRegistryMirrors) } label: {
            HStack(spacing: WontonTheme.spacing.s8) {
                if mirror.testing { ProgressView().controlSize(.small) }
                Text(TabText.choose("测速", "Test speed"))
            }
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
        .disabled(mirror.testing)
    }

    private func results(_ settings: RegistryMirrorSettings) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            // Tests only cover the core's candidates and saved address, never an unsaved draft.
            if let tests = mirror.tests {
                ForEach(tests.mirrors, id: \.mirror) { result in
                    if !enabled || (!settings.choices.contains(result.mirror) &&
                        ((!custom && !removedChoice) || draft != result.mirror)) {
                        RegistryProbeRow(title: result.mirror, probe: result.probe)
                    }
                }
            }
            RegistryProbeRow(title: TabText.choose("Docker Hub 直连", "Docker Hub direct"),
                             probe: mirror.tests == nil ? settings.dockerHub : mirror.tests?.dockerHub)
        }
    }

    private func probe(_ address: String) -> RegistryProbe? {
        mirror.tests?.mirrors.first { $0.mirror == address }?.probe
    }

    private func synchronize(_ settings: RegistryMirrorSettings?) {
        guard let settings else { return }
        // Probe/recommendation/choice refreshes must not erase a draft or a failed save.
        guard !initialized || savedMirror != settings.mirror else { return }
        initialized = true
        savedMirror = settings.mirror
        enabled = settings.mirror != nil
        selection = settings.mirror
        custom = settings.mirror.map { !settings.choices.contains($0) } ?? false
        customAddress = custom ? settings.mirror ?? "" : ""
    }
}

private struct RegistryProbeLabel: View {
    let probe: RegistryProbe?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .trailing, spacing: WontonTheme.spacing.s4) {
            Text(label)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(probe?.problem == nil ? colors.textSecondary : colors.danger))
            if let problem = probe?.problem {
                ProblemDetail(text: problem.message, minimumTargetHeight: WontonTheme.size.target)
            }
        }
    }

    private var label: String {
        guard let probe else { return TabText.choose("不确定", "Unknown") }
        if let problem = probe.problem {
            switch problem.reason {
            case .unreachable: return TabText.choose("连不上", "Unreachable")
            case .rejected: return TabText.choose("被拒绝", "Rejected")
            case .digestMismatch: return TabText.choose("摘要不符", "Digest mismatch")
            case .other: return TabText.choose("其他错误", "Other failure")
            }
        }
        if let latency = probe.latencyMs { return "\(latency) ms" }
        return TabText.choose("可用", "Usable")
    }
}

private struct RegistryProbeRow: View {
    let title: String
    let probe: RegistryProbe?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Text(verbatim: title)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            Spacer(minLength: 0)
            RegistryProbeLabel(probe: probe)
        }
        .padding(WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
    }
}

enum RegistryMirrorText {
    static var title: String { TabText.choose("镜像加速", "Image mirrors") }
    static var custom: String { TabText.choose("自定义", "Custom") }
    static var recommended: String {
        TabText.choose("Docker Hub 连不上，建议打开镜像加速。",
                       "Docker Hub is unreachable. Consider enabling image mirrors.")
    }
    static var risk: String {
        TabText.choose("这些是第三方公益服务，能看到拉了哪些镜像，但改不了镜像内容，因为摘要会逐一核对。",
                       "These are free third-party services. They can see which images you pull, " +
                       "but they can't change them: every image's digest is checked.")
    }
}
