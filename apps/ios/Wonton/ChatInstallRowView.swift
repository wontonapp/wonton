import SwiftUI
import WontonClient

/// 聊天列表最上面的安装行（ADR-0067）：正在装、正在升级、装失败或者结果不确定。只渲染核心给的行，
/// 重试交给确认页，来源不知道时引导去应用页。
struct ChatInstallRowView: View {
    let row: BotInstallRow
    let plugins: PluginsModel
    let retry: () -> Void
    let openPage: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: row.icon, size: WontonTheme.size.avatar, model: plugins)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                title
                progressLine
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .accessibilityElement(children: .contain)
    }

    private var title: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            Text(row.title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
            Spacer(minLength: 0)
            if case let .running(progress) = row.state {
                if let percent = progress.percent {
                    Text(Self.percentText(percent))
                        .wontonText(WontonTheme.typography.caption)
                        .monospacedDigit()
                        .foregroundStyle(Color(colors.textSecondary))
                } else {
                    ProgressView().controlSize(.small)
                }
            }
        }
    }

    @ViewBuilder private var progressLine: some View {
        switch row.state {
        case let .running(progress):
            Text(Self.stageText(progress.stage, operation: row.operation))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if let percent = progress.percent {
                ProgressView(value: Double(percent), total: 100)
            }
        case let .failed(reason, message):
            failure(reason, message: message)
        case let .uncertain(message):
            Text(.pluginTaskUncertain(message))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.warning))
            actions
        }
    }

    private func failure(_ reason: PluginFailure, message: String) -> some View {
        let problem = reason.problem(detail: message)
        let sentence = switch row.operation {
        case .install: String(localized: .pluginTaskInstallFailed(problem.title))
        case .upgrade: String(localized: .pluginTaskUpgradeFailed(problem.title))
        case .uninstall: String(localized: .pluginTaskUninstallFailed(problem.title))
        }
        return VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(sentence)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.danger))
            if let nextStep = problem.nextStep {
                Text(nextStep)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            ProblemDetail(text: message)
            actions
        }
    }

    /// 升级直接重新预览；装的时候来源不知道就不能重试，回应用页看它。
    private var canRetry: Bool { row.operation == .upgrade || row.source != nil }

    private var actions: some View {
        Button(canRetry ? String(localized: .commonRetry) : String(localized: .chatInstallOpenPage)) {
            if canRetry { retry() } else { openPage() }
        }
        .buttonStyle(WontonSmallButtonStyle())
    }

    /// 阶段由核心给，不认识的阶段落到这个操作通用的进行时。
    static func stageText(_ stage: InstallStage, operation: PluginOperation) -> String {
        switch stage {
        case .pullingImages: String(localized: .chatInstallStagePullingImages)
        case .preparing: String(localized: .chatInstallStagePreparing)
        case .starting: String(localized: .chatInstallStageStarting)
        case .waitingForBot: String(localized: .chatInstallStageWaitingForBot)
        default: operation.runningText
        }
    }

    static func percentText(_ percent: UInt8) -> String {
        (Double(percent) / 100).formatted(.percent)
    }
}
