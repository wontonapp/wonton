import SwiftUI
import WontonClient

/// “添加设备”：向家里的服务器要一个一次性配对码，画成二维码给新设备上的云吞 App 扫。进展和码都来自核心的
/// `HomeViewModel.invite`。
struct InviteView: View {
    @Bindable var model: HomeModel
    let chat: ChatModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                // 配对码正出示给新设备时，新设备发来的确认请求就在这一屏接着显示，不另弹（ADR-0046）。
                if let flow = chat.viewModel.encryption.verificationFlow, !flow.startedHere {
                    if giving, case .done = flow.stage {
                        Text(.Invite.inviteDeviceAdded)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                    } else {
                        DeviceVerificationFlowView(flow: flow, model: chat)
                    }
                }
                if chat.viewModel.encryption.keyHandoff == .cannotGive {
                    WontonNotice(icon: .warning, title: String(localized: .Invite.inviteCannotGive))
                }
                // 码已经被新设备用掉、这一页在替它交出密钥（ADR-0087）：不再摆码。
                if !giving {
                    code
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Invite.inviteTitle), usesSystemToolbar: true)
        .task { model.send(.home(action: .inviteDevice)) }
        .onDisappear { model.send(.home(action: .closeDeviceInvite)) }
    }

    private var giving: Bool { chat.viewModel.encryption.keyHandoff == .giving }

    @ViewBuilder private var code: some View {
        switch model.viewModel.invite {
        case nil, .requesting:
            HStack(spacing: WontonTheme.spacing.s12) {
                ProgressView()
                Text(.Invite.inviteRequesting)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
        case let .ready(code, expiresAtMs):
            InviteCode(code: code, expiresAt: Date(timeIntervalSince1970: Double(expiresAtMs) / 1000)) {
                model.send(.home(action: .inviteDevice))
            }
        case let .failed(_, message):
            WontonNotice(icon: .warning, title: String(localized: .Invite.inviteFailedTitle), lines: [message]) {
                Button(String(localized: .Invite.inviteTryAgain)) { model.send(.home(action: .inviteDevice)) }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
    }
}

/// 码的二维码、文字和剩下的有效期；过期后换一个。
private struct InviteCode: View {
    let code: String
    let expiresAt: Date
    let renew: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let remaining = Int(expiresAt.timeIntervalSince(context.date))
            if remaining > 0 {
                VStack(spacing: WontonTheme.spacing.s12) {
                    if let image = QRImage.make(code) {
                        Image(uiImage: image)
                            .interpolation(.none)
                            .resizable()
                            .scaledToFit()
                            .frame(maxWidth: 260)
                            .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
                            .accessibilityLabel(String(localized: .Invite.inviteQrLabel))
                    }
                    Text(.Invite.inviteExpiresIn(
                        Duration.seconds(TimeInterval(remaining)).formatted(.time(pattern: .minuteSecond))
                    ))
                        .wontonText(WontonTheme.typography.supporting)
                        .monospacedDigit()
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .frame(maxWidth: .infinity)
            } else {
                WontonNotice(icon: .warning, title: String(localized: .Invite.inviteExpired)) {
                    Button(String(localized: .Invite.inviteRenew), action: renew)
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        }
        Text(.Invite.inviteInstructions)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.Invite.invitePasteFallback)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            Text(code)
                .font(.caption.monospaced())
                // 英文断词会给这段码加上连字符，看着像码的一部分。
                .typesettingLanguage(.init(identifier: "zh-Hans"))
                .foregroundStyle(Color(colors.textPrimary))
                .textSelection(.enabled)
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
        }
    }
}
