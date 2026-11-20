import SwiftUI
import WontonClient

/// 输入栏里停下 Bot 这一轮的圆钮（ADR-0061 第 10 节）。核心说有轮次在跑才出现；请求在路上时转圈、点不了，
/// 减弱动态效果时不转。没停成、不确定时按钮还在，再点就是重试，原因在输入栏上方的 `AgentStopNotice`。
struct AgentStopButton: View {
    let state: RoomAgentStop
    let action: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    /// 这个状态下要不要画按钮：没有在跑的、刚停下的都不画。
    static func isShown(_ state: RoomAgentStop) -> Bool {
        switch state {
        case .running, .stopping, .failed, .uncertain: true
        case .idle, .stopped: false
        }
    }

    private var isStopping: Bool {
        if case .stopping = state { true } else { false }
    }

    var body: some View {
        Button(action: action) {
            mark
                .frame(width: WontonTheme.size.sendButton, height: WontonTheme.size.sendButton)
                .background(Circle().fill(Color(colors.primary)))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isStopping)
        .accessibilityLabel(Text(isStopping ? .Composer.composerStopping : .Composer.composerStop))
        .accessibilityIdentifier("composer-stop")
    }

    /// 停止的方块是画出来的形状，不是图标资源。
    @ViewBuilder private var mark: some View {
        if isStopping, !reduceMotion {
            ProgressView().controlSize(.small).tint(Color(colors.onPrimary))
        } else {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(Color(colors.onPrimary))
                .frame(width: WontonTheme.size.iconTiny, height: WontonTheme.size.iconTiny)
                .opacity(isStopping ? WontonTheme.ratio.secondaryContent : 1)
        }
    }
}

/// 输入栏上方的一行：停的结果。停成了说一声，核心过几秒自己收走；没停成、不确定按类别说，原因收在“详情”里。
struct AgentStopNotice: View {
    let state: RoomAgentStop
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch state {
        case .stopped:
            line(.Composer.composerStopped).modifier(Inset()).accessibilityIdentifier("composer-stop-stopped")
        case let .failed(error):
            VStack(alignment: .leading, spacing: 0) {
                line(Self.isRefusal(error) ? .Composer.composerStopForbidden : .Composer.composerStopFailed)
                ProblemDetail(text: String(describing: error))
            }
            .modifier(Inset())
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("composer-stop-failed")
        case let .uncertain(error):
            VStack(alignment: .leading, spacing: 0) {
                line(.Composer.composerStopUncertain)
                ProblemDetail(text: String(describing: error))
            }
            .modifier(Inset())
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("composer-stop-uncertain")
        case .idle, .running, .stopping:
            EmptyView()
        }
    }

    private func line(_ text: LocalizedStringResource) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 留白跟着内容走：没有可说的时候这一行不占地方。
    private struct Inset: ViewModifier {
        func body(content: Content) -> some View {
            content
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.top, WontonTheme.spacing.s8)
        }
    }

    /// 身份对、但这一轮不让这个人停：再点也一样，文字不叫人重试。
    private static func isRefusal(_ error: CoreError) -> Bool {
        switch error {
        case .Forbidden, .Unauthorized: true
        default: false
        }
    }
}
