import SwiftUI

/// 扫对方的码：取景框扫，扫不到就粘贴。原文原样交给 `onCode`，是用户 ID 还是 matrix.to 链接由核心认，
/// 认不出来时好友页按类别说。
struct FriendScannerView: View {
    let onCode: (String) -> Void
    @State private var pastedCode = ""
    /// 码进画面报一次，移出再进来会再报；交过一次就不再交。
    @State private var submitted = false

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            QRCameraArea(unsupported: .Peers.qrScannerUnsupported, onCode: submit)
            VStack(spacing: WontonTheme.spacing.s8) {
                TextField(String(localized: .Friends1.friendsPasteCode), text: $pastedCode, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(pastedCode.isEmpty ? nil : .caption.monospaced())
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(1...5)
                    .wontonField()
                Button {
                    submit(pastedCode)
                } label: {
                    Text(.Friends3.friendsAddSubmit).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(pastedCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(WontonTheme.spacing.pageInset)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func submit(_ code: String) {
        guard !submitted else { return }
        submitted = true
        onCode(code)
    }
}

/// 我的二维码：核心给的 matrix.to 用户链接画成码，对方扫了或粘贴链接就是发好友请求。链接长按可复制。
struct MyFriendCodePanel: View {
    let link: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            if let image = QRImage.make(link) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 260)
                    .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
                    .accessibilityLabel(String(localized: .Friends3.friendsMyCode))
            }
            Text(link)
                .font(.caption.monospaced())
                .foregroundStyle(Color(colors.textPrimary))
                .textSelection(.enabled)
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
                .accessibilityIdentifier("friends-my-link")
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s16)
    }
}
