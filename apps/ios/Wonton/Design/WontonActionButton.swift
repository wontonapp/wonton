import SwiftUI
import WontonClient

/// 破坏性的动作先确认：写清会怎样，“取消”在前。
struct WontonActionConfirm {
    let title: String
    var message: String?
    let confirm: String
}

/// 让核心去做一件事的按钮（ADR-0026 的“动作按钮的反馈”）。按下变淡并轻震一下；`run` 是核心给的进度：
/// 进行中转圈、不能再点，没做成时由同一处的 `WontonActionNotice` 说一句人话。破坏性的给 `confirm`，先确认再做。
/// 壳不自己记点没点过、做没做成，进度只听核心的。
struct WontonActionButton: View {
    enum Size { case regular, small }

    let title: String
    var kind: WontonButtonStyle.Kind = .secondary
    var size: Size = .regular
    var run: ActionRun = .idle
    var confirm: WontonActionConfirm?
    /// 界面测试按它找按钮；确认框的两个按钮是 `<identifier>.confirm` 和 `<identifier>.cancel`。
    var identifier: String?
    let action: () -> Void
    @State private var taps = 0
    @State private var confirming = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let button = styled(Button(action: tapped) { label })
            .disabled(running)
            .accessibilityValue(running ? String(localized: .Design.designActionRunning) : "")
            .accessibilityIdentifier(identifier ?? "")
            .sensoryFeedback(.impact(weight: .light), trigger: taps)
            .sensoryFeedback(trigger: run.phase) { _, phase in phase.feedback }
        if let confirm {
            button.wontonDialog(
                confirm.title,
                message: confirm.message,
                isPresented: $confirming,
                actions: [
                    WontonDialogAction(
                        title: String(localized: .Common.commonCancel),
                        identifier: identifier.map { "\($0).cancel" }
                    ),
                    WontonDialogAction(
                        title: confirm.confirm, kind: .danger, perform: action,
                        identifier: identifier.map { "\($0).confirm" }
                    )
                ]
            )
        } else {
            button
        }
    }

    private var running: Bool { run.phase == .running }

    private var label: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            if running {
                ProgressView()
                    .controlSize(.small)
                    .tint(Color(ink))
            }
            Text(title)
        }
    }

    @ViewBuilder private func styled(_ button: Button<some View>) -> some View {
        switch size {
        case .regular: button.buttonStyle(WontonButtonStyle(kind: kind))
        case .small: button.buttonStyle(WontonSmallButtonStyle(kind: kind == .danger ? .danger : .secondary))
        }
    }

    private var ink: Rgb {
        switch kind {
        case .primary: colors.onPrimary
        case .secondary: colors.textPrimary
        case .danger: colors.danger
        }
    }

    private func tapped() {
        guard !running else { return }
        taps += 1
        if confirm != nil { confirming = true } else { action() }
    }
}

/// 动作没做成或不确定时的那一句，放在按钮所在的那一组下面；原文收进默认收起的“详情”。别的进度不占地方。
struct WontonActionNotice: View {
    let run: ActionRun
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch run {
        case let .failed(failure, details):
            notice(icon: .warning, tint: colors.danger, text: failure.text, details: details)
        case .uncertain:
            notice(icon: .info, tint: colors.textSecondary,
                   text: String(localized: .Design.designActionUncertain), details: nil)
        case .idle, .running, .succeeded, .other:
            EmptyView()
        }
    }

    private func notice(icon: WontonIcon, tint: Rgb, text: String, details: String?) -> some View {
        VStack(spacing: WontonTheme.spacing.s4) {
            HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s4) {
                icon.view(size: WontonTheme.size.iconTiny).foregroundStyle(Color(tint))
                Text(text)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("actionNotice")
            if let details { ProblemDetail(text: details, toggleIdentifier: "actionNoticeDetails") }
        }
    }
}

/// 按进度给的触觉：成了、没成各震一种，进行中和回到空闲不震。
enum ActionPhase: Equatable {
    case idle, running, succeeded, failed, uncertain

    var feedback: SensoryFeedback? {
        switch self {
        case .succeeded: .success
        case .failed: .error
        case .uncertain: .warning
        case .idle, .running: nil
        }
    }
}

extension ActionRun {
    var phase: ActionPhase {
        switch self {
        case .running: .running
        case .succeeded: .succeeded
        case .failed: .failed
        case .uncertain: .uncertain
        case .idle, .other: .idle
        }
    }
}

extension ActionFailure {
    /// 一句人话，按类别写；原文在详情里。
    var text: String {
        switch self {
        case .transport: String(localized: .Design.designActionFailedTransport)
        case .permission: String(localized: .Design.designActionFailedPermission)
        case .notReady: String(localized: .Design.designActionFailedNotReady)
        case .upstream: String(localized: .Design.designActionFailedUpstream)
        case .stillFailing: String(localized: .Design.designActionFailedStill)
        case .invalidInput: String(localized: .Design.designActionFailedInvalid)
        case .other: String(localized: .Design.designActionFailedOther)
        }
    }
}
