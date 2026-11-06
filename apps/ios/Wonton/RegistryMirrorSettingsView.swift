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
            .wontonPushedScreen(String(localized: .registryMirrorTitle)) {
                if mirror.saving {
                    ProgressView().padding(.trailing, WontonTheme.spacing.s12)
                } else {
                    Button(String(localized: .commonSave)) {
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
                    Text(.registryMirrorRecommended)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.warning))
                }
                if let problem = mirror.loadProblem {
                    ProblemNotice(problem: problem.problem)
                    Button(String(localized: .commonRetry)) { model.send(.refreshRegistryMirror) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        .disabled(mirror.loading)
                }
                if let problem = mirror.saveProblem {
                    ProblemNotice(problem: problem.reason == .invalid
                                  ? Problem(title: String(localized: .registryMirrorSaveFailed),
                                            detail: problem.message)
                                  : problem.saveProblem)
                }
                if let problem = mirror.testProblem { ProblemNotice(problem: problem.problem) }
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    Toggle(String(localized: .registryMirrorTitle), isOn: $enabled)
                        .toggleStyle(WontonToggleStyle())
                    Text(.registryMirrorRisk)
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
            StepOptionRow(title: String(localized: .registryMirrorCustom), isSelected: custom || removedChoice) {
                if removedChoice { customAddress = customValue }
                custom = true
            }
            if custom || removedChoice {
                TextField(String(localized: .registryMirrorCustom), text: Binding(
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
                Text(.registryMirrorTest)
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
            RegistryProbeRow(title: String(localized: .registryMirrorDockerHubDirect),
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
        guard let probe else { return String(localized: .registryMirrorProbeUnknown) }
        if let problem = probe.problem {
            switch problem.reason {
            case .unreachable: return String(localized: .registryMirrorProbeUnreachable)
            case .rejected: return String(localized: .registryMirrorProbeRejected)
            case .digestMismatch: return String(localized: .registryMirrorProbeDigestMismatch)
            case .other: return String(localized: .registryMirrorProbeOther)
            }
        }
        if let latency = probe.latencyMs {
            return Measurement(value: Double(latency), unit: UnitDuration.milliseconds)
                .formatted(.measurement(width: .abbreviated))
        }
        return String(localized: .registryMirrorProbeAvailable)
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
