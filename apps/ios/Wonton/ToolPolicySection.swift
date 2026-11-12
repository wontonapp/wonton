import SwiftUI
import WontonClient

/// 插件的工具权限（ADR-0024）：放行中的时段、始终允许、一律允许、哪些工具算只读。开关打开时不再弹确认，风险写在旁边。
struct ToolPolicySection: View {
    let model: PluginsModel
    let id: String
    let version: String
    @State private var policy: ToolPolicyView?
    @State private var problem: Problem?
    @State private var busy = false

    var body: some View {
        content
            .task(id: "\(id) \(version)") { await load() }
            .task(id: policy?.windows.map(\.expiresAtMs).min()) { await reloadAtExpiry() }
    }

    @ViewBuilder private var content: some View {
        if let policy {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let reason = policy.problem {
                    ProblemNotice(
                        problem: Problem(
                            title: String(localized: .ToolPolicy.toolPolicyUnreadableTitle),
                            nextStep: String(localized: .ToolPolicy.toolPolicyUnreadableNext),
                            detail: reason
                        )
                    )
                }
                if let problem { ProblemNotice(problem: problem) }
                sections(policy, enabled: !busy && policy.problem == nil)
            }
        } else if let problem {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                ProblemNotice(problem: problem)
                Button(String(localized: .Common.commonRetry)) { Task { await load() } }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .fixedSize()
            }
        } else {
            ProgressView()
        }
    }

    @ViewBuilder private func sections(_ policy: ToolPolicyView, enabled: Bool) -> some View {
        OpenWindows(windows: policy.windows, enabled: !busy) { card in
            change { try await model.core.stopToolWindow(id: id, card: card) }
        }
        BotSwitches(
            title: String(localized: .ToolPolicy.toolPolicyAlwaysTitle),
            risk: String(localized: .ToolPolicy.toolPolicyAlwaysRisk),
            bots: policy.always,
            enabled: enabled
        ) { caller, isOn in
            change { try await model.core.setToolAlways(id: id, caller: caller, enabled: isOn) }
        }
        BotSwitches(
            title: String(localized: .ToolPolicy.toolPolicyAllowAllTitle),
            risk: String(localized: .ToolPolicy.toolPolicyAllowAllRisk),
            bots: policy.allowAll,
            enabled: enabled
        ) { caller, isOn in
            change { try await model.core.setToolAllowAll(id: id, caller: caller, enabled: isOn) }
        }
        ToolList(tools: policy.tools, error: policy.toolsError, enabled: enabled) { tool, isOn in
            change { try await model.core.setToolReadOnly(id: id, tool: tool, enabled: isOn) }
        }
    }

    private func load() async {
        do {
            policy = try await model.core.toolPolicy(id: id)
            problem = nil
        } catch {
            problem = error.problem
        }
    }

    /// 到点的时段不再生效，这一行要跟着消失。
    private func reloadAtExpiry() async {
        guard let next = policy?.windows.map(\.expiresAtMs).min() else { return }
        let wait = Date(timeIntervalSince1970: Double(next) / 1000).timeIntervalSinceNow
        try? await Task.sleep(for: .seconds(max(wait, 0)))
        if !Task.isCancelled { await load() }
    }

    private func change(_ call: @escaping () async throws -> ToolPolicyView) {
        busy = true
        problem = nil
        Task {
            do {
                policy = try await call()
            } catch {
                problem = error.problem
                // 请求可能已经到了核心，开关以核心现在的为准。
                if let current = try? await model.core.toolPolicy(id: id) { policy = current }
            }
            busy = false
        }
    }
}

private struct OpenWindows: View {
    let windows: [OpenWindow]
    let enabled: Bool
    let stop: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if !windows.isEmpty {
            DetailSection(title: String(localized: .ToolPolicy.toolPolicyWindowsTitle)) {
                ForEach(windows, id: \.card) { window in
                    HStack(spacing: WontonTheme.spacing.s12) {
                        Text(.ToolPolicy.toolPolicyWindowUntil(window.name, until(window)))
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(String(localized: .ToolPolicy.toolPolicyStop)) { stop(window.card) }
                            .buttonStyle(WontonButtonStyle(kind: .secondary))
                            .fixedSize()
                            .disabled(!enabled)
                    }
                }
            }
        }
    }

    private func until(_ window: OpenWindow) -> String {
        Date(timeIntervalSince1970: Double(window.expiresAtMs) / 1000).formatted(date: .omitted, time: .shortened)
    }
}

struct BotSwitches: View {
    let title: String
    let risk: String
    let bots: [BotSwitch]
    let enabled: Bool
    let change: (String, Bool) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if !bots.isEmpty {
            DetailSection(title: title) {
                Text(risk)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.warning))
                ForEach(bots, id: \.caller) { bot in
                    Toggle(isOn: Binding(get: { bot.enabled }, set: { change(bot.caller, $0) })) {
                        Text(bot.name)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                    .toggleStyle(WontonToggleStyle())
                    .disabled(!enabled)
                }
            }
        }
    }
}

private struct ToolList: View {
    let tools: [ToolPolicyEntry]
    let error: String?
    let enabled: Bool
    let change: (String, Bool) -> Void
    @State private var expanded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if tools.isEmpty {
            if let error {
                DetailSection(title: String(localized: .ToolPolicy.toolPolicyToolsTitle)) {
                    WontonNotice(icon: .warning, title: error)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Button { expanded.toggle() } label: {
                    HStack {
                        Text(.ToolPolicy.toolPolicyToolsTitle)
                            .wontonText(WontonTheme.typography.sectionTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                        Spacer(minLength: 0)
                        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.textSecondary))
                            .rotationEffect(expanded ? .degrees(90) : .zero)
                    }
                    .frame(minHeight: WontonTheme.size.target)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(.isHeader)
                if expanded {
                    Text(.ToolPolicy.toolPolicyReadOnlyHint)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.warning))
                    ForEach(tools, id: \.name) { ToolRow(tool: $0, enabled: enabled, change: change) }
                }
            }
        }
    }
}

/// 插件自己标的只读主人改不了：这里只放宽。
private struct ToolRow: View {
    let tool: ToolPolicyEntry
    let enabled: Bool
    let change: (String, Bool) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: 0) {
                Text(tool.name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let description = tool.description {
                    Text(description)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if tool.access == .pluginReadOnly {
                Text(.ToolPolicy.toolPolicyReadOnlyTag)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                // WontonToggleStyle 自己画标签，`labelsHidden` 对它不起作用。
                Toggle(
                    isOn: Binding(
                        get: { tool.access == .ownerReadOnly },
                        set: { change(tool.name, $0) }
                    )
                ) { EmptyView() }
                .accessibilityLabel(tool.name)
                .toggleStyle(WontonToggleStyle())
                .disabled(!enabled)
                .fixedSize()
            }
        }
    }
}
