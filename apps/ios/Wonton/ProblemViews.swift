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
                title: String(localized: .problemPluginSaveUncertainTitle),
                nextStep: String(localized: .problemPluginSaveUncertainNextStep),
                detail: message
            )
        case .invalid:
            return Problem(title: String(localized: .problemPluginSaveInvalidTitle), nextStep: message)
        default:
            return problem
        }
    }
}

extension PluginFailure {
    func problem(detail: String?) -> Problem {
        switch self {
        case .transportUnavailable:
            categorizedProblem(.problemTransportUnavailableTitle, .problemTransportUnavailableNextStep, detail)
        case .unauthorized: categorizedProblem(.problemUnauthorizedTitle, .problemUnauthorizedNextStep, detail)
        case .upstream: categorizedProblem(.problemUpstreamTitle, .problemUpstreamNextStep, detail)
        case .sourceUnavailable:
            categorizedProblem(.problemPluginSourceUnavailableTitle, .problemPluginSourceUnavailableNextStep, detail)
        case .notFound: categorizedProblem(.problemPluginNotFoundTitle, .problemPluginNotFoundNextStep, detail)
        case .changed: categorizedProblem(.problemPluginChangedTitle, .problemPluginChangedNextStep, detail)
        case .unsupported: categorizedProblem(.problemPluginUnsupportedTitle, .problemUpgradeServerNextStep, detail)
        case .conflict: categorizedProblem(.problemPluginConflictTitle, .problemPluginConflictNextStep, detail)
        case .invalid: categorizedProblem(.problemPluginInvalidTitle, .problemPluginInvalidNextStep, detail)
        case .deviceUnavailable:
            categorizedProblem(.problemPluginDeviceUnavailableTitle, .problemPluginDeviceUnavailableNextStep, detail)
        case .lanUnavailable:
            categorizedProblem(.problemPluginLanUnavailableTitle, .problemPluginLanUnavailableNextStep, detail)
        case .lanPortTaken:
            categorizedProblem(.problemPluginLanPortTakenTitle, .problemPluginLanPortTakenNextStep, detail)
        case .coreUpgrading:
            categorizedProblem(.problemPluginCoreUpgradingTitle, .problemPluginCoreUpgradingNextStep, detail)
        case .egressProxyUnavailable:
            categorizedProblem(
                .problemPluginEgressProxyUnavailableTitle, .problemPluginEgressProxyUnavailableNextStep, detail
            )
        case .protocol: categorizedProblem(.problemPluginProtocolTitle, .problemPluginProtocolNextStep, detail)
        case .serverTooOld:
            categorizedProblem(.problemPluginServerTooOldTitle, .problemUpgradeServerNextStep, detail)
        // 比这个壳新的类别：先按服务器出错说。
        default: categorizedProblem(.problemUpstreamTitle, .problemUpstreamNextStep, detail)
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
            title: String(localized: .problemAppearanceTitle),
            nextStep: String(localized: .problemStorageNextStep),
            detail: message
        )
    }
}

extension ChatFailure {
    func problem(detail: String) -> Problem {
        switch self {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .problemTransportUnavailableTitle),
                nextStep: String(localized: .problemTransportUnavailableNextStep),
                detail: detail
            )
        case .unauthorized:
            return Problem(
                title: String(localized: .problemUnauthorizedTitle),
                nextStep: String(localized: .problemUnauthorizedNextStep),
                detail: detail
            )
        case .upstream:
            return Problem(
                title: String(localized: .problemUpstreamTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
                detail: detail
            )
        case .notReady:
            return Problem(title: String(localized: .problemNotReadyTitle), nextStep: detail)
        case .invalidInput:
            return Problem(title: String(localized: .problemInvalidInputTitle), nextStep: detail)
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
                title: String(localized: .problemTransportTitle),
                nextStep: String(localized: .problemTransportNext),
                detail: detail
            )
        case .notReady:
            return Problem(
                title: String(localized: .botProfileProblemNotReadyTitle),
                nextStep: String(localized: .botProfileProblemNotReadyNext),
                detail: detail
            )
        case let .invalidName(reason):
            return Problem(title: reason.problem)
        case let .mediaRejected(reason):
            return Problem(title: reason.problem)
        case .upstream:
            return Problem(
                title: String(localized: .problemUpstreamTitle),
                nextStep: String(localized: .problemUpstreamNext),
                detail: detail
            )
        // 家人设备调只有主人的接口（ADR-0045）：不是出错，也不是还没准备好。
        case .ownerOnly:
            return Problem(
                title: String(localized: .botProfileProblemOwnerOnlyTitle),
                nextStep: String(localized: .botProfileProblemOwnerOnlyNext),
                detail: detail
            )
        case .other:
            return Problem(
                title: String(localized: .botProfileProblemOtherTitle),
                nextStep: String(localized: .problemUpstreamNext),
                detail: detail
            )
        }
    }
}

extension BotNameProblem {
    var problem: String {
        switch self {
        case .empty: String(localized: .botProfileProblemNameEmpty)
        case .tooLong: String(localized: .botProfileProblemNameTooLong)
        case .controlCharacters: String(localized: .botProfileProblemNameControlCharacters)
        case .reserved: String(localized: .botProfileProblemNameReserved)
        case .other: String(localized: .botProfileProblemNameOther)
        }
    }
}

extension AvatarMediaProblem {
    var problem: String {
        switch self {
        case .format: String(localized: .botProfileProblemMediaFormat)
        case .tooLarge: String(localized: .botProfileProblemMediaTooLarge)
        case .tooBig: String(localized: .botProfileProblemMediaTooBig)
        case .notSquare: String(localized: .botProfileProblemMediaNotSquare)
        case .tooLong: String(localized: .botProfileProblemMediaTooLong)
        case .unreadable: String(localized: .botProfileProblemMediaUnreadable)
        case .other: String(localized: .botProfileProblemMediaOther)
        }
    }
}

extension StoreProblem {
    var problem: Problem {
        switch reason {
        case .unreachable:
            return Problem(title: String(localized: .problemStoreUnreachableTitle), detail: details)
        case .timeout:
            return Problem(title: String(localized: .problemStoreTimeoutTitle), detail: details)
        case .rateLimited:
            return Problem(
                title: String(localized: .problemStoreRateLimitedTitle),
                nextStep: String(localized: .problemStoreRateLimitedNextStep),
                detail: details
            )
        case .invalidIndex:
            return Problem(
                title: String(localized: .problemStoreInvalidIndexTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
                detail: details
            )
        case .indexTooNew:
            return Problem(
                title: String(localized: .problemStoreIndexTooNewTitle),
                nextStep: String(localized: .problemUpgradeServerNextStep),
                detail: details
            )
        case .upstream:
            return Problem(
                title: String(localized: .problemStoreUpstreamTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
                detail: details
            )
        case .incomplete:
            return Problem(title: String(localized: .problemStoreIncompleteTitle), detail: details)
        case .disabled:
            return Problem(title: String(localized: .problemStoreDisabledTitle), detail: details)
        case .other:
            return Problem(title: String(localized: .problemStoreOtherTitle), detail: details)
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
                title: String(localized: .problemGenericTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
                detail: detail
            )
        }
        switch error {
        case .TransportUnavailable:
            return Problem(
                title: String(localized: .problemTransportUnavailableTitle),
                nextStep: String(localized: .problemTransportUnavailableNextStep),
                detail: detail
            )
        case .Unauthorized:
            return Problem(
                title: String(localized: .problemUnauthorizedTitle),
                nextStep: String(localized: .problemUnauthorizedNextStep),
                detail: detail
            )
        case .Upstream:
            return Problem(
                title: String(localized: .problemUpstreamTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
                detail: detail
            )
        case let .NotReady(reason):
            return Problem(title: String(localized: .problemNotReadyTitle), nextStep: reason)
        case .Storage:
            return Problem(
                title: String(localized: .problemStorageTitle),
                nextStep: String(localized: .problemStorageNextStep),
                detail: detail
            )
        case let .InvalidInput(reason):
            return Problem(title: String(localized: .problemInvalidInputTitle), nextStep: reason)
        case .ServerTooOld:
            return Problem(
                title: String(localized: .problemPluginServerTooOldTitle),
                nextStep: String(localized: .problemUpgradeServerNextStep),
                detail: detail
            )
        // 比这个壳新的类别：先按服务器出错说。
        default:
            return Problem(
                title: String(localized: .problemUpstreamTitle),
                nextStep: String(localized: .problemUpstreamNextStep),
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
                    Text(.commonDetails)
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
            Button(String(localized: .commonRetry), action: retry)
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
                    .accessibilityLabel(String(localized: .commonClose))
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
                Button(String(localized: .commonRetry), action: retry).buttonStyle(WontonSmallButtonStyle())
            }
        }
    }
}

/// 过了几秒还没好时才出现的一句话。
private struct SlowHint: View {
    @State private var isSlow = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(.problemSlowLoading)
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
        .accessibilityLabel(Text(.problemLoading))
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
