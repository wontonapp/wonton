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
        case .high: String(localized: .pluginPermissionRiskHigh)
        case .veryHigh: String(localized: .pluginPermissionRiskVeryHigh)
        case .highest: String(localized: .pluginPermissionRiskHighest)
        case .other: String(localized: .pluginPermissionRiskUnknown)
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
                String(localized: .pluginPermissionUnderstandTitle),
                message: active?.line.text,
                isPresented: $hostPresented,
                actions: [
                    WontonDialogAction(title: String(localized: .commonCancel), perform: clear),
                    WontonDialogAction(title: String(localized: .pluginPermissionOpen), kind: .danger) {
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
            ? String(localized: .pluginPermissionOpenTitle(active.line.text))
            : String(localized: .pluginPermissionCloseTitle(active.line.text))
    }

    private var firstActions: [WontonDialogAction] {
        let needsHost = active.map { $0.enabled && $0.line.requiresHostConfirmation } ?? false
        let accept = needsHost ? String(localized: .pluginPermissionContinue)
            : (active?.enabled == true ? String(localized: .pluginPermissionOpen)
                : String(localized: .pluginPermissionClose))
        return [
            WontonDialogAction(title: String(localized: .commonCancel), perform: clear),
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
