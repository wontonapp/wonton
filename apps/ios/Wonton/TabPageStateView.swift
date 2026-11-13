import SwiftUI
import WontonClient

struct TabPageStateView: View {
    let page: TabPage
    let session: TabWebSession
    let connected: Bool
    let shareProblem: Problem?
    /// 面板里的这一份：失败时多一个“从面板移除”（ADR-0052）。
    let isPanel: Bool
    let details: () -> Void
    let remove: () -> Void
    @State private var startProblem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            // widget 页（ADR-0096）要壳接上驱动的订阅才说得上话，这一版的壳没有：如实说打不开。
            if case .widget = session.tab {
                failure(String(localized: .Tab0.tabWidgetUnsupported)) {
                    Button(TabText.close, action: remove)
                }
            } else if let problem = shareProblem ?? session.problem {
                VStack {
                    ProblemView(problem: problem, retry: session.retry)
                    if case .sharedApp = session.tab {
                        Button(TabText.close, action: remove).buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color(colors.canvas))
            } else {
                state
            }
        }
    }

    @ViewBuilder private var state: some View {
        switch page.state {
        case .ready:
            if !session.ready { skeleton() }
        case .loading:
            skeleton()
        case .starting:
            skeleton(TabText.starting)
        case .stopped:
            failure(TabText.stopped(page.title)) {
                if case .plugin = session.tab {
                    Button(TabText.start, action: startPlugin)
                    if let startProblem { ProblemNotice(problem: startProblem) }
                } else {
                    Button(TabText.retry, action: session.retry)
                    Button(TabText.close, action: remove)
                }
            }
        case .upgrading:
            if session.webView != nil {
                VStack {
                    Text(TabText.upgrading)
                        .wontonText(WontonTheme.typography.supporting)
                        .padding(WontonTheme.spacing.s8)
                        .background(.regularMaterial, in: Capsule())
                    Spacer(minLength: 0)
                }
                .allowsHitTesting(false)
            } else { skeleton(TabText.upgrading) }
        case .uninstalling:
            skeleton(TabText.uninstalling)
        case let .failed(reason):
            failure(String(localized: .Tab1.tabFailedTitle(page.title)), detail: reason) {
                if case .plugin = session.tab { Button(TabText.details, action: details) }
                Button(TabText.retry, action: session.retry)
                if isPanel { Button(TabText.removePanelPage, action: remove) }
            }
        case .pageRemoved:
            failure(TabText.pageRemoved) {
                Button(isPanel ? TabText.removePanelPage : TabText.close, action: remove)
            }
        case let .loadFailed(reason, status):
            if reason == .transport, !connected {
                if !session.ready { skeleton() }
            } else {
                failure(TabText.pageFailed, detail: status.map { "HTTP \($0)" }, next: reason.label) {
                    Button(TabText.retry, action: session.retry)
                    if isPanel { Button(TabText.removePanelPage, action: remove) }
                }
            }
        case .unknown:
            failure(TabText.unknown) {
                if case .plugin = session.tab { Button(TabText.details, action: details) }
                Button(TabText.retry, action: session.retry)
                if isPanel { Button(TabText.removePanelPage, action: remove) }
            }
        }
    }

    private func startPlugin() {
        let pluginId: String
        switch session.tab {
        case let .plugin(id, _), let .pluginPath(id, _): pluginId = id
        default: return
        }
        do {
            try session.core.dispatch(action: .startPlugin(pluginId: pluginId))
            startProblem = nil
        } catch { startProblem = error.problem }
    }

    private func skeleton(_ title: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(0..<4, id: \.self) { _ in
                RoundedRectangle(cornerRadius: WontonTheme.radii.card)
                    .fill(Color(colors.surfaceSubtle))
                    .frame(height: WontonTheme.size.target)
            }
            if let title {
                Text(title).wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Spacer(minLength: 0)
        }
        .padding(WontonTheme.spacing.pageInset)
        .background(Color(colors.canvas))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title ?? TabText.loading)
        .allowsHitTesting(false)
    }

    private func failure<Buttons: View>(
        _ title: String, detail: String? = nil, next: String? = nil, @ViewBuilder buttons: () -> Buttons
    ) -> some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            WontonIcon.warning.view().foregroundStyle(Color(colors.textSecondary))
            Text(title).wontonText(WontonTheme.typography.itemTitle)
            if let next { Text(next).wontonText(WontonTheme.typography.supporting) }
            buttons().buttonStyle(WontonButtonStyle(kind: .secondary))
            if let detail { ProblemDetail(text: detail) }
        }
        .foregroundStyle(Color(colors.textPrimary))
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(colors.canvas))
    }
}

private extension TabLoadFailure {
    var label: String {
        switch self {
        case .transport: String(localized: .Tab1.tabFailureTransport)
        case .pluginNotReady: String(localized: .Tab2.tabFailurePluginNotReady)
        case .upstream: String(localized: .Tab1.tabFailureUpstream)
        }
    }
}
