import SwiftUI
import WontonClient

/// 卸载前的确认（ADR-0053）：先问核心能不能卸。能卸时列出可以一起卸的、没人再用的提供方；
/// 被在用的依赖挡住时列出依赖方，给出“一起卸载”或“先停它们”。清失败的安装时只有一个按钮。
struct UninstallSheet: View {
    let target: UninstallTarget
    let model: PluginsModel
    let close: (@escaping () -> Void) -> Void
    @State private var outcome: UninstallOutcome?
    @State private var problem: Problem?
    @State private var chosen: Set<String> = []
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .task { await load() }
    }

    @ViewBuilder private var content: some View {
        if let problem {
            VStack(spacing: WontonTheme.spacing.s12) {
                ProblemNotice(problem: problem)
                Button(String(localized: .Common.commonRetry)) {
                    Task { await load() }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                cancelButton
            }
        } else {
            switch outcome {
            case nil:
                ProgressView().padding(WontonTheme.spacing.s24)
            case let .ready(unusedProviders):
                ready(unusedProviders)
            case let .inUse(dependents):
                inUse(dependents)
            default:
                ProblemNotice(problem: Problem(title: String(localized: .Problem2.problemUpstreamTitle)))
            }
        }
    }

    private func ready(_ unused: [CapabilityProviderRef]) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if !target.keepDataOnly {
                Text(.Plugin1.pluginUninstallDeleteDataWarning)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            if !unused.isEmpty {
                Text(.Plugin2.pluginUninstallUnusedProviders)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                ForEach(unused, id: \.id) { provider in
                    StepOptionRow(
                        title: provider.title,
                        isSelected: chosen.contains(provider.id),
                        neutralSelection: true,
                        select: { toggle(provider.id) },
                        leading: { EmptyView() },
                        trailing: { EmptyView() }
                    )
                    .accessibilityAddTraits(chosen.contains(provider.id) ? .isSelected : [])
                }
            }
            if target.keepDataOnly {
                Button(String(localized: .Plugin0.pluginClearFailure), role: .destructive) {
                    close { uninstall(keepData: true, dependents: []) }
                }
                .buttonStyle(WontonButtonStyle(kind: .danger))
            } else {
                Button(String(localized: .Plugin1.pluginUninstallKeepData)) {
                    close { uninstall(keepData: true, dependents: []) }
                }
                .buttonStyle(WontonButtonStyle())
                Button(String(localized: .Plugin1.pluginUninstallDeleteData), role: .destructive) {
                    close { uninstall(keepData: false, dependents: []) }
                }
                .buttonStyle(WontonButtonStyle(kind: .danger))
            }
            cancelButton
        }
    }

    private func inUse(_ dependents: [CapabilityProviderRef]) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            WontonNotice(
                icon: .warning,
                title: String(localized: .Plugin3.pluginUninstallInUse(target.title)),
                lines: dependents.map(\.title)
            )
            Button(String(localized: .Plugin0.pluginUninstallTogether)) {
                close { uninstall(keepData: true, dependents: dependents.map(\.id)) }
            }
            .buttonStyle(WontonButtonStyle())
            Button(String(localized: .Plugin1.pluginUninstallStopDependents)) {
                close {
                    for dependent in dependents {
                        model.send(.plugins(action: .stopPlugin(pluginId: dependent.id)))
                    }
                }
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            cancelButton
        }
    }

    private var cancelButton: some View {
        Button(String(localized: .Common.commonCancel)) { close {} }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
    }

    private func toggle(_ id: String) {
        if chosen.contains(id) {
            chosen.remove(id)
        } else {
            chosen.insert(id)
        }
    }

    private func uninstall(keepData: Bool, dependents: [String]) {
        model.send(.plugins(action: .uninstallPlugin(
            id: target.id,
            deleteData: !keepData,
            providers: Array(chosen),
            dependents: dependents
        )))
    }

    private func load() async {
        problem = nil
        outcome = nil
        do {
            outcome = try await model.core.uninstallPreview(id: target.id)
        } catch {
            problem = error.problem
        }
    }
}

extension View {
    /// 卸载前确认：先问核心要预览，挡住时给选择，不挡时列出可以一起卸的提供方。
    func uninstallDialog(_ target: Binding<UninstallTarget?>, model: PluginsModel) -> some View {
        let value = target.wrappedValue
        return wontonSheet(
            isPresented: Binding(
                get: { target.wrappedValue != nil },
                set: { isShowing in
                    if !isShowing { target.wrappedValue = nil }
                }
            ),
            title: value?.keepDataOnly == true
                ? String(localized: .Plugin3.pluginClearFailureTitle(value?.title ?? ""))
                : String(localized: .Plugin2.pluginUninstallTitle(value?.title ?? ""))
        ) { close in
            if let value {
                UninstallSheet(target: value, model: model, close: close)
            }
        }
    }
}
