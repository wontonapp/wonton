import SwiftUI
import WontonClient

struct PermissionChange: Identifiable {
    let line: PermissionLine
    let enabled: Bool
    var id: String { line.permission }
}

extension PermissionLine {
    var requiresHostConfirmation: Bool {
        switch risk {
        case .high, .veryHigh: false
        case .highest, .other, nil: true
        }
    }

    var riskLabel: String? {
        switch risk {
        case .high: String(localized: .Plugin0.pluginPermissionRiskHigh)
        case .veryHigh: String(localized: .Plugin2.pluginPermissionRiskVeryHigh)
        case .highest: String(localized: .Plugin1.pluginPermissionRiskHighest)
        case .other: String(localized: .Plugin2.pluginPermissionRiskUnknown)
        case nil: nil
        }
    }
}

extension View {
    func permissionConfirmation(
        _ change: Binding<PermissionChange?>,
        confirmed: @escaping (PermissionChange) -> Void
    ) -> some View {
        modifier(PermissionConfirmation(change: change, confirmed: confirmed))
    }
}

private struct PermissionConfirmation: ViewModifier {
    @Binding var change: PermissionChange?
    let confirmed: (PermissionChange) -> Void
    @State private var firstPresented = false
    @State private var hostPresented = false
    @State private var pendingHost: PermissionChange?
    @State private var active: PermissionChange?

    func body(content: Content) -> some View {
        content
            .onChange(of: change?.id) { _, _ in
                guard let change else { return }
                active = change
                firstPresented = true
            }
            .wontonDialog(
                title,
                message: active.map { $0.enabled ? $0.line.notice : $0.line.offLoss } ?? nil,
                isPresented: $firstPresented,
                actions: firstActions,
                onDismiss: clear,
                afterDismiss: {
                    if let pendingHost {
                        active = pendingHost
                        self.pendingHost = nil
                        hostPresented = true
                    }
                }
            )
            .wontonDialog(
                String(localized: .Plugin3.pluginPermissionUnderstandTitle),
                message: active?.line.text,
                isPresented: $hostPresented,
                actions: [
                    WontonDialogAction(title: String(localized: .Common.commonCancel), perform: clear),
                    WontonDialogAction(title: String(localized: .Plugin2.pluginPermissionOpen), kind: .danger) {
                        if let active { confirmed(active) }
                        clear()
                    }
                ],
                onDismiss: clear
            )
    }

    private var title: String {
        guard let active else { return "" }
        return active.enabled
            ? String(localized: .Plugin2.pluginPermissionOpenTitle(active.line.text))
            : String(localized: .Plugin3.pluginPermissionCloseTitle(active.line.text))
    }

    private var firstActions: [WontonDialogAction] {
        let needsHost = active.map { $0.enabled && $0.line.requiresHostConfirmation } ?? false
        let accept = needsHost ? String(localized: .Plugin0.pluginPermissionContinue)
            : (active?.enabled == true ? String(localized: .Plugin2.pluginPermissionOpen)
                : String(localized: .Plugin0.pluginPermissionClose))
        return [
            WontonDialogAction(title: String(localized: .Common.commonCancel), perform: clear),
            WontonDialogAction(title: accept, kind: .primary) {
                guard let active else { return }
                if needsHost {
                    pendingHost = active
                } else {
                    confirmed(active)
                    clear()
                }
            }
        ]
    }

    private func clear() {
        change = nil
        active = nil
        pendingHost = nil
    }
}
