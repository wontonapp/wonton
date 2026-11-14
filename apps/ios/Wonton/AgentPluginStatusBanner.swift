import SwiftUI
import WontonClient

/// 只读核心给房间投影的插件状态，不以已安装列表推断房间关联。
struct AgentPluginStatusBanner: View {
    let plugin: RoomPlugin
    let model: PluginsModel

    var body: some View {
        Group {
            if plugin.start != nil || plugin.status == .stopped {
                PluginStartStatusView(
                    model: model, pluginId: plugin.pluginId, state: plugin.start,
                    canStart: plugin.canStart, isAgent: true
                )
            } else {
                status
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }

    @ViewBuilder private var status: some View {
        switch plugin.status {
        case .running, .stopped:
            EmptyView()
        case .installing:
            WontonNotice(icon: .info, title: String(localized: .Plugin2.pluginTaskInstallRunning))
        case .upgrading:
            WontonNotice(icon: .info, title: TabText.upgrading)
        case .uninstalling:
            WontonNotice(icon: .info, title: TabText.uninstalling)
        case let .degraded(reason):
            WontonNotice(icon: .warning, title: TabText.failed) {
                ProblemDetail(text: reason, minimumTargetHeight: WontonTheme.size.target)
            }
        case .needsConfiguration:
            WontonNotice(icon: .wrench, title: String(localized: .Launcher.launcherBadgeNeedsConfiguration))
        case .unknown:
            WontonNotice(icon: .warning, title: TabText.unknown)
        }
    }
}

/// 聊天和详情共用的启动反馈；异步结果来自核心，分发被拒只在当前入口显示。
struct PluginStartStatusView: View {
    let model: PluginsModel
    let pluginId: String
    let state: PluginStartState?
    let canStart: Bool
    var isAgent = false
    @State private var rejection: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if case .starting = state {
                HStack(spacing: WontonTheme.spacing.s8) {
                    ProgressView()
                    Text(isAgent ? String(localized: .Chat0.chatPluginStarting) : TabText.starting)
                        .wontonText(WontonTheme.typography.supporting)
                }
            } else if let problem {
                failure(problem)
            } else {
                HStack(spacing: WontonTheme.spacing.s12) {
                    WontonIcon.pauseFill.view(size: WontonTheme.size.iconSmall)
                    Text(isAgent ? String(localized: .Chat1.chatPluginStopped)
                         : String(localized: .Launcher.launcherBadgeStopped))
                        .wontonText(WontonTheme.typography.supporting)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if canStart { startButton(TabText.start) }
                }
            }
        }
        .foregroundStyle(Color(colors.textPrimary))
        .padding(.vertical, WontonTheme.spacing.s8)
        .onChange(of: state) { _, _ in rejection = nil }
    }

    private var problem: Problem? {
        if let rejection { return rejection }
        if case let .failed(problem) = state { return problem.problem }
        return nil
    }

    private func failure(_ problem: Problem) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(problem.title).wontonText(WontonTheme.typography.supporting)
            if let nextStep = problem.nextStep {
                Text(nextStep)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            if canStart { startButton(TabText.retry) }
            if let detail = problem.detail {
                ProblemDetail(text: detail, minimumTargetHeight: WontonTheme.size.target)
                    .id(detail)
            }
        }
    }

    private func startButton(_ title: String) -> some View {
        Button(title) {
            do {
                try model.core.dispatch(action: .plugins(action: .startPlugin(pluginId: pluginId)))
                rejection = nil
            } catch { rejection = error.problem }
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
        .frame(minHeight: WontonTheme.size.target)
    }
}
