import SwiftUI
import WontonClient

struct TabPageStateView: View {
    let page: TabPage
    let session: TabWebSession
    let connected: Bool
    let details: () -> Void
    let remove: () -> Void
    @State private var startProblem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            if let problem = session.problem {
                ProblemView(problem: problem, retry: session.retry)
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
                Button(TabText.start, action: startPlugin)
                if let startProblem { ProblemNotice(problem: startProblem) }
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
            failure("\(page.title) \(TabText.failed)", detail: reason) {
                Button(TabText.details, action: details)
                Button(TabText.retry, action: session.retry)
            }
        case .pageRemoved:
            failure(TabText.pageRemoved) { Button(TabText.unpin, action: remove) }
        case let .loadFailed(reason, status):
            if reason == .transport, !connected {
                if !session.ready { skeleton() }
            } else {
                failure(TabText.pageFailed, detail: status.map { "HTTP \($0)" }, next: reason.label) {
                    Button(TabText.retry, action: session.retry)
                }
            }
        case .unknown:
            failure(TabText.unknown) { Button(TabText.details, action: details) }
        }
    }

    private func startPlugin() {
        guard case let .plugin(pluginId, _) = session.tab else { return }
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
        case .transport: TabText.choose("连不上家里的服务器", "Cannot connect to your server")
        case .pluginNotReady: TabText.choose("应用还没准备好", "The app is not ready")
        case .upstream: TabText.choose("应用返回了错误", "The app returned an error")
        }
    }
}
