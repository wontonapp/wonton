import SwiftUI

/// 附件面板选了照片、转发面板选了会话以后，面板底下的那一条（ADR-0109 第 9 节）：左边一块玻璃的附言框，
/// 右边重点色的圆形发送键，键角上标选了几个。贴着面板底，键盘弹起时跟着上来。
struct CaptionSendBar: View {
    let placeholder: String
    @Binding var text: String
    let count: Int
    /// 有的话最左边多一个关闭钮：不发了，回到选之前。
    var cancel: (() -> Void)?
    /// 现在能不能发；不给就随时能发（转发只要求选了会话）。
    var canSend: Bool?
    let send: () -> Void
    /// 附言框、发送键、关闭钮的无障碍标识前缀，如 `attach` 得到 `attachCaption`、`attachSend`、`attachCancel`。
    let id: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .bottom, spacing: WontonTheme.spacing.s4) {
            if let cancel {
                ChatGlassButton(icon: .close, label: String(localized: .Common.commonCancel), action: cancel)
                    .accessibilityIdentifier("\(id)Cancel")
            }
            field
            ChatGlassButton(icon: .send, label: sendLabel, prominent: true, action: send)
                .overlay(alignment: .topTrailing) { badge }
                .opacity(canSend == false ? WontonTheme.ratio.secondaryContent : 1)
                .disabled(canSend == false)
                .accessibilityIdentifier("\(id)Send")
        }
        // 不放进一个玻璃容器：容器里的玻璃会盖住压在发送键上的角标。
        .padding(.horizontal, WontonTheme.spacing.chatFloatInset - WontonTheme.spacing.s4)
        .padding(.bottom, WontonTheme.spacing.s4)
    }

    private var field: some View {
        let shape = RoundedRectangle(cornerRadius: WontonTheme.size.chatControl / 2, style: .continuous)
        return TextField(placeholder, text: $text, axis: .vertical)
            .lineLimit(1...Int(WontonTheme.ratio.composerLines))
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s8)
            .frame(minHeight: WontonTheme.size.chatControl)
            // 实底，不用玻璃：面板本身在 iOS 26 上就是玻璃，玻璃叠玻璃在深色里几乎看不出框。和转发面板的搜索框同一个底。
            .background(Color(colors.surfaceSubtle), in: shape)
            .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.chatControl) / 2)
            .accessibilityIdentifier("\(id)Caption")
    }

    private var sendLabel: String {
        "\(String(localized: .Composer.composerSend)) (\(count))"
    }

    /// 选了几个：压在发送键右上角的小胶囊，描一圈底色和键分开。
    private var badge: some View {
        Text(verbatim: "\(count)")
            .wontonText(WontonTheme.typography.caption)
            .fontWeight(.semibold)
            .monospacedDigit()
            .foregroundStyle(Color(colors.canvas))
            .padding(.horizontal, WontonTheme.spacing.s4)
            .frame(minWidth: WontonTheme.size.iconSmall, minHeight: WontonTheme.size.iconSmall)
            .background(Capsule().fill(Color(colors.accentInk)))
            .overlay(Capsule().strokeBorder(Color(colors.canvas), lineWidth: 1.5))
            .accessibilityHidden(true)
    }
}
