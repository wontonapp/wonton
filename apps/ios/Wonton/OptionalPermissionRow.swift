import SwiftUI
import WontonClient

struct OptionalPermissionRow: View {
    let model: PluginsModel
    let id: String
    let line: PermissionLine
    let updated: (PluginDetail) -> Void
    @State private var busy = false
    @State private var problem: Problem?
    @State private var failedTarget: Bool?
    @State private var change: PermissionChange?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            PermissionRow(line: line, selection: Binding(
                get: { line.enabled },
                set: { change = PermissionChange(line: line, enabled: $0) }
            ), busy: busy)
            if line.enabled, let opened = line.opened { openedLine(opened) }
            if let problem {
                WontonNotice(
                    icon: .warning,
                    title: problem.title,
                    lines: problem.nextStep.map { [$0] } ?? []
                ) {
                    Button(String(localized: .commonRetry)) {
                        guard let failedTarget else { return }
                        Task { await apply(failedTarget) }
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .disabled(busy)
                    if let detail = problem.detail { ProblemDetail(text: detail) }
                }
            }
        }
        .permissionConfirmation($change) { change in
            Task { await apply(change.enabled) }
        }
    }

    private func apply(_ enabled: Bool) async {
        guard !busy else { return }
        busy = true
        problem = nil
        failedTarget = nil
        do {
            updated(try await model.core.setPluginPermission(id: id, permission: line.permission, enabled: enabled))
        } catch {
            problem = error.problem
            failedTarget = enabled
            if let detail = try? await model.core.installedPlugin(id: id) { updated(detail) }
        }
        busy = false
    }

    private func openedLine(_ opened: PermissionOpened) -> some View {
        let date = Date(timeIntervalSince1970: TimeInterval(opened.since))
            .formatted(.dateTime.month(.defaultDigits).day())
        let text = opened.bot.map {
            String(localized: .pluginPermissionOpenedBot($0.title, date))
        } ?? String(localized: .pluginPermissionOpenedOwner(date))
        return Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
    }
}
