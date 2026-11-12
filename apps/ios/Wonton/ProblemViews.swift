import SwiftUI
import WontonClient

/// 给人看的问题：`title` 说发生了什么，`nextStep` 说能做什么，`detail` 是核心写的技术细节，默认收起。类别由核心给，
/// 文字按类别写在这里。
struct Problem {
    let title: String
    var nextStep: String?
    var detail: String?
}

extension PluginProblem {
    var problem: Problem { reason.problem(detail: message) }
}

extension PluginProblem {
    /// 保存出网代理没成功：连不上时不知道到底改了没有，填错了要照着核心说的改。
    var saveProblem: Problem {
        switch reason {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem1.problemPluginSaveUncertainTitle),
                nextStep: String(localized: .Problem2.problemPluginSaveUncertainNextStep),
                detail: message
            )
        case .invalid:
            return Problem(title: String(localized: .Problem0.problemPluginSaveInvalidTitle), nextStep: message)
        default:
            return problem
        }
    }
}

extension PluginFailure {
    func problem(detail: String?) -> Problem {
        switch self {
        case .transportUnavailable:
            categorizedProblem(
                .Problem2.problemTransportUnavailableTitle,
                .Problem3.problemTransportUnavailableNextStep,
                detail)
        case .unauthorized:
            categorizedProblem(
                .Problem3.problemUnauthorizedTitle,
                .Problem2.problemUnauthorizedNextStep,
                detail)
        case .upstream: categorizedProblem(.Problem2.problemUpstreamTitle, .Problem2.problemUpstreamNextStep, detail)
        case .sourceUnavailable:
            categorizedProblem(
                .Problem1.problemPluginSourceUnavailableTitle,
                .Problem0.problemPluginSourceUnavailableNextStep,
                detail)
        case .notFound:
            categorizedProblem(
                .Problem2.problemPluginNotFoundTitle,
                .Problem0.problemPluginNotFoundNextStep,
                detail)
        case .changed:
            categorizedProblem(
                .Problem3.problemPluginChangedTitle,
                .Problem1.problemPluginChangedNextStep,
                detail)
        case .unsupported:
            categorizedProblem(
                .Problem2.problemPluginUnsupportedTitle,
                .Problem0.problemUpgradeServerNextStep,
                detail)
        case .conflict:
            categorizedProblem(
                .Problem1.problemPluginConflictTitle,
                .Problem3.problemPluginConflictNextStep,
                detail)
        case .invalid:
            categorizedProblem(
                .Problem2.problemPluginInvalidTitle,
                .Problem3.problemPluginInvalidNextStep,
                detail)
        case .deviceUnavailable:
            categorizedProblem(
                .Problem1.problemPluginDeviceUnavailableTitle,
                .Problem3.problemPluginDeviceUnavailableNextStep,
                detail)
        case .lanUnavailable:
            categorizedProblem(
                .Problem0.problemPluginLanUnavailableTitle,
                .Problem1.problemPluginLanUnavailableNextStep,
                detail)
        case .lanPortTaken:
            categorizedProblem(
                .Problem2.problemPluginLanPortTakenTitle,
                .Problem2.problemPluginLanPortTakenNextStep,
                detail)
        case .coreUpgrading:
            categorizedProblem(
                .Problem3.problemPluginCoreUpgradingTitle,
                .Problem2.problemPluginCoreUpgradingNextStep,
                detail)
        case .egressProxyUnavailable:
            categorizedProblem(
                .Problem3.problemPluginEgressProxyUnavailableTitle,
                .Problem1.problemPluginEgressProxyUnavailableNextStep,
                detail
            )
        case .protocol:
            categorizedProblem(
                .Problem0.problemPluginProtocolTitle,
                .Problem0.problemPluginProtocolNextStep,
                detail)
        case .recordUnusable:
            categorizedProblem(
                .Problem0.problemPluginRecordUnusableTitle,
                .Problem0.problemPluginRecordUnusableNextStep,
                detail)
        case .serverTooOld:
            categorizedProblem(.Problem3.problemPluginServerTooOldTitle, .Problem0.problemUpgradeServerNextStep, detail)
        // 比这个壳新的类别：先按服务器出错说。
        default: categorizedProblem(.Problem2.problemUpstreamTitle, .Problem2.problemUpstreamNextStep, detail)
        }
    }
}

/// 连不上、不认设备、服务器出错这三句在插件、聊天和连接错误里共用。
private func categorizedProblem(
    _ title: LocalizedStringResource,
    _ nextStep: LocalizedStringResource,
    _ detail: String?
) -> Problem {
    Problem(title: String(localized: title), nextStep: String(localized: nextStep), detail: detail)
}

extension AppearanceProblem {
    var problem: Problem {
        Problem(
            title: String(localized: .Problem1.problemAppearanceTitle),
            nextStep: String(localized: .Problem1.problemStorageNextStep),
            detail: message
        )
    }
}

extension ChatFailure {
    func problem(detail: String) -> Problem {
        switch self {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: detail
            )
        case .unauthorized:
            return Problem(
                title: String(localized: .Problem3.problemUnauthorizedTitle),
                nextStep: String(localized: .Problem2.problemUnauthorizedNextStep),
                detail: detail
            )
        case .upstream:
            return Problem(
                title: String(localized: .Problem2.problemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: detail
            )
        case .notReady:
            return Problem(title: String(localized: .Problem0.problemNotReadyTitle), nextStep: detail)
        case .invalidInput:
            return Problem(title: String(localized: .Problem2.problemInvalidInputTitle), nextStep: detail)
        }
    }
}

extension ChatProblem {
    var problem: Problem { reason.problem(detail: message) }
}

extension BotProfileFailure {
    /// 改资料没成：类别由核心给，文字写在这里（ADR-0043）。名字和文件的细节是单独的键。
    func problem(detail: String) -> Problem {
        switch self {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem1.problemTransportTitle),
                nextStep: String(localized: .Problem0.problemTransportNext),
                detail: detail
            )
        case .notReady:
            return Problem(
                title: String(localized: .BotProfile0.botProfileProblemNotReadyTitle),
                nextStep: String(localized: .BotProfile0.botProfileProblemNotReadyNext),
                detail: detail
            )
        case let .invalidName(reason):
            return Problem(title: reason.problem)
        case let .mediaRejected(reason):
            return Problem(title: reason.problem)
        case .upstream:
            return Problem(
                title: String(localized: .Problem2.problemUpstreamTitle),
                nextStep: String(localized: .Problem1.problemUpstreamNext),
                detail: detail
            )
        // 家人设备调只有主人的接口（ADR-0045）：不是出错，也不是还没准备好。
        case .ownerOnly:
            return Problem(
                title: String(localized: .BotProfile0.botProfileProblemOwnerOnlyTitle),
                nextStep: String(localized: .BotProfile0.botProfileProblemOwnerOnlyNext),
                detail: detail
            )
        case .other:
            return Problem(
                title: String(localized: .BotProfile1.botProfileProblemOtherTitle),
                nextStep: String(localized: .Problem1.problemUpstreamNext),
                detail: detail
            )
        }
    }
}

extension BotNameProblem {
    var problem: String {
        switch self {
        case .empty: String(localized: .BotProfile0.botProfileProblemNameEmpty)
        case .tooLong: String(localized: .BotProfile1.botProfileProblemNameTooLong)
        case .controlCharacters: String(localized: .BotProfile1.botProfileProblemNameControlCharacters)
        case .reserved: String(localized: .BotProfile1.botProfileProblemNameReserved)
        case .other: String(localized: .BotProfile0.botProfileProblemNameOther)
        }
    }
}

extension AvatarMediaProblem {
    var problem: String {
        switch self {
        case .format: String(localized: .BotProfile0.botProfileProblemMediaFormat)
        case .tooLarge: String(localized: .BotProfile1.botProfileProblemMediaTooLarge)
        case .tooBig: String(localized: .BotProfile1.botProfileProblemMediaTooBig)
        case .notSquare: String(localized: .BotProfile0.botProfileProblemMediaNotSquare)
        case .tooLong: String(localized: .BotProfile1.botProfileProblemMediaTooLong)
        case .unreadable: String(localized: .BotProfile0.botProfileProblemMediaUnreadable)
        case .other: String(localized: .BotProfile1.botProfileProblemMediaOther)
        }
    }
}

extension StoreProblem {
    var problem: Problem {
        switch reason {
        case .unreachable:
            return Problem(title: String(localized: .Problem3.problemStoreUnreachableTitle), detail: details)
        case .timeout:
            return Problem(title: String(localized: .Problem0.problemStoreTimeoutTitle), detail: details)
        case .rateLimited:
            return Problem(
                title: String(localized: .Problem1.problemStoreRateLimitedTitle),
                nextStep: String(localized: .Problem1.problemStoreRateLimitedNextStep),
                detail: details
            )
        case .invalidIndex:
            return Problem(
                title: String(localized: .Problem0.problemStoreInvalidIndexTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: details
            )
        case .indexTooNew:
            return Problem(
                title: String(localized: .Problem3.problemStoreIndexTooNewTitle),
                nextStep: String(localized: .Problem0.problemUpgradeServerNextStep),
                detail: details
            )
        case .upstream:
            return Problem(
                title: String(localized: .Problem3.problemStoreUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: details
            )
        case .incomplete:
            return Problem(title: String(localized: .Problem1.problemStoreIncompleteTitle), detail: details)
        case .disabled:
            return Problem(title: String(localized: .Problem0.problemStoreDisabledTitle), detail: details)
        case .other:
            return Problem(title: String(localized: .Problem0.problemStoreOtherTitle), detail: details)
        }
    }

    /// 主人自己关掉的，重试也不会变。
    var canRetry: Bool { reason != .disabled }
}

extension Error {
    var problem: Problem {
        let detail = String(describing: self)
        guard let error = self as? CoreError else {
            return Problem(
                title: String(localized: .Problem1.problemGenericTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: detail
            )
        }
        switch error {
        case .TransportUnavailable:
            return Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: detail
            )
        case .Unauthorized:
            return Problem(
                title: String(localized: .Problem3.problemUnauthorizedTitle),
                nextStep: String(localized: .Problem2.problemUnauthorizedNextStep),
                detail: detail
            )
        case .Upstream:
            return Problem(
                title: String(localized: .Problem2.problemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: detail
            )
        case let .NotReady(reason):
            return Problem(title: String(localized: .Problem0.problemNotReadyTitle), nextStep: reason)
        case .Storage:
            return Problem(
                title: String(localized: .Problem3.problemStorageTitle),
                nextStep: String(localized: .Problem1.problemStorageNextStep),
                detail: detail
            )
        case let .InvalidInput(reason):
            return Problem(title: String(localized: .Problem2.problemInvalidInputTitle), nextStep: reason)
        case .ServerTooOld:
            return Problem(
                title: String(localized: .Problem3.problemPluginServerTooOldTitle),
                nextStep: String(localized: .Problem0.problemUpgradeServerNextStep),
                detail: detail
            )
        // 比这个壳新的类别：先按服务器出错说。
        default:
            return Problem(
                title: String(localized: .Problem2.problemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: detail
            )
        }
    }
}

/// 收起的技术细节。排查时才看，能选中复制。
struct ProblemDetail: View {
    let text: String
    var minimumTargetHeight: CGFloat?
    @State private var isExpanded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Button {
                isExpanded.toggle()
            } label: {
                HStack(spacing: WontonTheme.spacing.s4) {
                    Text(.Common.commonDetails)
                    WontonIcon.forward.view(size: WontonTheme.size.iconTiny)
                        .rotationEffect(isExpanded ? .degrees(90) : .zero)
                }
                .frame(minHeight: minimumTargetHeight)
                .foregroundStyle(Color(colors.textSecondary))
            }
            .buttonStyle(.plain)
            if isExpanded {
                Text(text)
                    .foregroundStyle(Color(colors.textSecondary))
                    .multilineTextAlignment(.leading)
                    .textSelection(.enabled)
            }
        }
        .wontonText(WontonTheme.typography.caption)
    }
}

/// 什么都显示不出来，整页说明。
struct ProblemView: View {
    let problem: Problem
    let retry: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.warning.view().foregroundStyle(Color(colors.textSecondary))
            Text(problem.title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if let nextStep = problem.nextStep {
                Text(nextStep)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Button(String(localized: .Common.commonRetry), action: retry)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .padding(.vertical, WontonTheme.spacing.s8)
            if let detail = problem.detail { ProblemDetail(text: detail) }
        }
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
    }
}

/// 页面还能用，只是这件事没做成。
struct ProblemNotice: View {
    let problem: Problem
    var icon: WontonIcon = .warning
    /// 这件事只关这一处显示时给关闭按钮；核心里的状态由调用方清。
    var onClose: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonNotice(icon: icon, title: problem.title, lines: problem.nextStep.map { [$0] } ?? []) {
            if let detail = problem.detail { ProblemDetail(text: detail) }
            if let onClose {
                HStack {
                    Spacer(minLength: 0)
                    Button(action: onClose) {
                        WontonIcon.close.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(String(localized: .Common.commonClose))
                }
            }
        }
    }
}

extension View {
    /// 主人点了什么没做成：说清原因就好，点“好”关掉。
    func wontonDialog(
        problem: Problem?,
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil
    ) -> some View {
        wontonDialog(
            problem?.title ?? "",
            message: problem?.nextStep,
            detail: problem?.detail,
            isPresented: isPresented,
            onDismiss: onDismiss
        )
    }
}

/// 列表还在，只是少了点什么或者不是最新的：一条轻提示，不抢列表的位置。
struct ProblemRow: View {
    let problem: Problem
    var retry: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            WontonIcon.info.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(problem.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
                if let nextStep = problem.nextStep {
                    Text(nextStep)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                if let detail = problem.detail { ProblemDetail(text: detail) }
            }
            Spacer(minLength: 0)
            if let retry {
                Button(String(localized: .Common.commonRetry), action: retry).buttonStyle(WontonSmallButtonStyle())
            }
        }
    }
}

/// 过了几秒还没好时才出现的一句话。
private struct SlowHint: View {
    @State private var isSlow = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(.Problem2.problemSlowLoading)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.center)
            .opacity(isSlow ? 1 : 0)
            .task {
                try? await Task.sleep(for: .seconds(5))
                isSlow = true
            }
    }
}

/// 取一个页面的内容：转圈，慢了说一句。
struct LoadingView: View {
    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            ProgressView()
            SlowHint()
        }
        .padding(WontonTheme.spacing.s32)
    }
}

/// 取列表：骨架占住列表的位置，慢了说一句。
struct ListSkeleton: View {
    let iconSize: CGFloat
    @State private var isDim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(0 ..< 5, id: \.self) { _ in row }
            SlowHint().frame(maxWidth: .infinity)
            Spacer(minLength: 0)
        }
        .padding(WontonTheme.spacing.pageInset)
        // 键盘弹起时放不下，从上往下裁，不要盖住上面的搜索框。
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .clipped()
        .opacity(isDim ? WontonTheme.ratio.secondaryContent : 1)
        .accessibilityLabel(Text(.Problem1.problemLoading))
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 0.9).repeatForever()) { isDim = true }
        }
    }

    private var row: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            RoundedRectangle(cornerRadius: iconSize * WontonTheme.ratio.pluginIconRadius, style: .continuous)
                .fill(Color(colors.surfaceSubtle))
                .frame(width: iconSize, height: iconSize)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                line.frame(maxWidth: 120)
                line
            }
        }
    }

    private var line: some View {
        Capsule().fill(Color(colors.surfaceSubtle)).frame(height: WontonTheme.spacing.s12)
    }
}
