import SwiftUI
import WontonClient

struct OnboardingReviewStep: View {
    let review: OnboardingReview
    let plugins: PluginsModel
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(review.items, id: \.id) { item in
                OnboardingReviewRow(item: item, plugins: plugins) {
                    send(.retryOnboardingItem(id: item.id))
                }
            }
        }
    }
}

private struct OnboardingReviewRow: View {
    let item: OnboardingReviewItem
    let plugins: PluginsModel
    let retry: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
                if let icon = item.icon {
                    PluginIconView(icon: icon, size: WontonTheme.size.taskIcon, model: plugins)
                }
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    Text(verbatim: item.title)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    if let detail = item.detail, !detail.isEmpty {
                        Text(verbatim: detail)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                progress
            }
            InstallPermissions(permissions: item.permissions, notices: item.notices)
            switch item.state {
            case let .blocked(message):
                Text(verbatim: message)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
            case let .uncertain(message):
                WontonNotice(icon: .warning, title: "不确定装好没有", isError: false, lines: ["到应用页查看安装结果。"]) {
                    ProblemDetail(text: message)
                }
            case let .failed(problem):
                ProblemNotice(problem: problem.onboardingProblem)
                Button("重试", action: retry).buttonStyle(WontonSmallButtonStyle())
            default:
                EmptyView()
            }
        }
        .padding(WontonTheme.spacing.s16)
        .background(Color(colors.surfaceSubtle), in: RoundedRectangle(cornerRadius: WontonTheme.radii.card))
    }

    @ViewBuilder private var progress: some View {
        switch item.state {
        case .preparing:
            ProgressView().accessibilityLabel("正在准备")
        case .waiting:
            Text("排队")
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case .running:
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text("进行中").wontonText(WontonTheme.typography.supporting)
            }
            .foregroundStyle(Color(colors.textSecondary))
        case .done:
            WontonIcon.check.view().foregroundStyle(Color(colors.success))
                .accessibilityLabel("已完成")
        case .ready, .blocked, .uncertain, .failed:
            EmptyView()
        }
    }
}

private extension OnboardingItemProblem {
    var onboardingProblem: Problem {
        switch self {
        case let .plugin(problem): problem.problem
        case let .model(problem): problem.onboardingProblem
        }
    }
}
