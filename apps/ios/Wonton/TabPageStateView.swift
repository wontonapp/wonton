import SwiftUI
import WontonClient

/// 页面容器的状态页：加载、启动中、停止、出了问题。出错页的按钮都走 `WontonActionButton`（ADR-0026 的“动作按钮的反馈”）：
/// 重试的进度和结果是核心按这一份实例给的（`TabPage.retry`）；“详情”在原地展开，不跳走。
struct TabPageStateView: View {
    let page: TabPage
    let session: TabWebSession
    let connected: Bool
    let shareProblem: Problem?
    /// 面板里的这一份：失败时多一个“从面板移除”（ADR-0052）。
    let isPanel: Bool
    let remove: () -> Void
    @State private var startProblem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            if let problem = shareProblem ?? session.problem {
                VStack {
                    ProblemView(problem: problem, retry: session.retryPage)
                    if case .sharedApp = session.tab {
                        WontonActionButton(title: TabText.close, action: remove)
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
                    WontonActionButton(title: TabText.start, identifier: "pageStart", action: startPlugin)
                    if let startProblem { ProblemNotice(problem: startProblem) }
                } else {
                    retryButton
                    WontonActionButton(title: TabText.close, action: remove)
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
                retryButton
                if isPanel { removeButton }
            }
        case .pageRemoved:
            failure(TabText.pageRemoved) {
                if isPanel { removeButton } else { WontonActionButton(title: TabText.close, action: remove) }
            }
        case let .loadFailed(reason, status, details):
            if reason == .transport, !connected {
                if !session.ready { skeleton() }
            } else {
                // 详情是核心按 App 语言写的（错误码收在里面）；核心没给时退回裸错误码。
                failure(TabText.pageFailed, detail: details ?? status.map { "HTTP \($0)" }, next: reason.label) {
                    retryButton
                    if isPanel { removeButton }
                }
            }
        case .unknown:
            failure(TabText.unknown) {
                retryButton
                if isPanel { removeButton }
            }
        }
    }

    private var retryButton: some View {
        WontonActionButton(title: TabText.retry, run: page.retry, identifier: "pageRetry", action: session.retryPage)
    }

    private var removeButton: some View {
        WontonActionButton(title: TabText.removePanelPage, action: remove)
    }

    private func startPlugin() {
        let pluginId: String
        switch session.tab {
        case let .plugin(id, _), let .pluginPath(id, _): pluginId = id
        default: return
        }
        do {
            try session.core.dispatch(action: .plugins(action: .startPlugin(pluginId: pluginId)))
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
            buttons()
            WontonActionNotice(run: page.retry)
            if let detail { ProblemDetail(text: detail, label: .Tab0.tabViewDetails, toggleIdentifier: "pageDetails") }
        }
        .foregroundStyle(Color(colors.textPrimary))
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(colors.canvas))
        .animation(.easeOut(duration: WontonTheme.motion.feedback), value: page.retry.phase)
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
