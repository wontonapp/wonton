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
                    Text(item.kind.section)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                    Text(item.titles.formatted(.list(type: .and)))
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    if let detail = item.detail {
                        Text(verbatim: detail.line)
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
                Text(verbatim: message ?? String(localized: .onboardingReviewBlocked))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
            case let .uncertain(message):
                WontonNotice(icon: .warning, title: String(localized: .onboardingReviewUncertain),
                             isError: false, lines: [String(localized: .onboardingReviewUncertainNext)]) {
                    ProblemDetail(text: message)
                }
            case let .failed(problem):
                ProblemNotice(problem: problem.onboardingProblem)
                Button(String(localized: .commonRetry), action: retry).buttonStyle(WontonSmallButtonStyle())
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
            ProgressView().accessibilityLabel(String(localized: .onboardingReviewPreparing))
        case .waiting:
            Text(.onboardingReviewWaiting)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case .running:
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text(.onboardingRunning).wontonText(WontonTheme.typography.supporting)
            }
            .foregroundStyle(Color(colors.textSecondary))
        case .done:
            WontonIcon.check.view().foregroundStyle(Color(colors.success))
                .accessibilityLabel(String(localized: .onboardingReviewDone))
        case .ready, .blocked, .uncertain, .failed:
            EmptyView()
        }
    }
}

/// 第二行的文字，按类别写在这里。
private extension OnboardingReviewDetail {
    var line: String {
        switch self {
        case let .email(email): email
        case .setAsDefault: String(localized: .onboardingReviewSetDefault)
        case .keepCurrent: String(localized: .onboardingReviewKeepCurrent)
        case let .model(model): model
        case let .unreachable(model):
            model.map { String(localized: .onboardingReviewUnreachableModel($0)) }
                ?? String(localized: .onboardingReviewUnreachable)
        case .signInAfterSetup: String(localized: .onboardingReviewSignInAfterSetup)
        case .unchanged: String(localized: .onboardingReviewUnchanged)
        }
    }
}

/// 每一行的类别，写在标题上面。
private extension OnboardingReviewKind {
    var section: String {
        switch self {
        case .profile: String(localized: .onboardingReviewSectionProfile)
        case .agent: String(localized: .onboardingReviewSectionAgent)
        case .model: String(localized: .onboardingReviewSectionModel)
        case .app: String(localized: .onboardingReviewSectionApps)
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
