import SwiftUI

/// 输入框右边“发送键那一格”：有字或有附件才出现发送键（ADR-0109 第 6 节），空着时不画东西；照片的字节还在取时这一格转圈。
/// 单独一个视图：别的按钮要占这一格（比如 Agent 在跑时的“停止”）就换在这里，不动输入框的布局。
struct ComposerSendSlot: View {
    let canSend: Bool
    /// 正在编辑消息：这一格是确认（勾），不是发送（ADR-0109 第 10 节）。
    var editing = false
    /// 附件的字节还在取：这一格换成转圈，等待期间还能接着打字。
    let isSending: Bool
    let send: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var showsSlot: Bool { canSend || isSending }

    var body: some View {
        ZStack {
            if isSending {
                ProgressView()
                    .frame(width: WontonTheme.size.chatControl, height: WontonTheme.size.chatControl)
                    .accessibilityIdentifier("composerSending")
            } else if canSend {
                SendButton(editing: editing, action: send)
                    .transition(reduceMotion ? .opacity : .scale.combined(with: .opacity))
            }
        }
        // 空着时这一格不占宽，输入框一直伸到右边。
        .padding(.leading, showsSlot ? 0 : -WontonTheme.spacing.s4)
        .animation(.easeOut(duration: WontonTheme.motion.feedback), value: showsSlot)
    }
}

private struct SendButton: View {
    let editing: Bool
    let action: () -> Void

    var body: some View {
        ChatGlassButton(
            icon: editing ? .check : .send,
            label: editing
                ? String(localized: .Composer.composerEditConfirm)
                : String(localized: .Composer.composerSend),
            prominent: true, action: action
        )
        .accessibilityIdentifier("composerSend")
    }
}
