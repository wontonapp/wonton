import CoreImage.CIFilterBuiltins
import SwiftUI
import WontonClient

/// “添加设备”：向家里的服务器要一个一次性配对码，画成二维码给新设备上的云吞 App 扫。进展和码都来自核心的
/// `HomeViewModel.invite`。
struct InviteView: View {
    @Bindable var model: HomeModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                switch model.viewModel.invite {
                case nil, .requesting:
                    HStack(spacing: WontonTheme.spacing.s12) {
                        ProgressView()
                        Text("正在向家里的服务器要配对码…")
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                case let .ready(code, expiresAtMs):
                    InviteCode(code: code, expiresAt: Date(timeIntervalSince1970: Double(expiresAtMs) / 1000)) {
                        model.send(.inviteDevice)
                    }
                case let .failed(_, message):
                    WontonNotice(icon: .warning, title: "没要到配对码", lines: [message]) {
                        Button("再试一次") { model.send(.inviteDevice) }
                            .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("添加设备")
        .task { model.send(.inviteDevice) }
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
                    }
                    Text("还有 \(remaining / 60):\(String(format: "%02d", remaining % 60)) 过期")
                        .wontonText(WontonTheme.typography.supporting)
                        .monospacedDigit()
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .frame(maxWidth: .infinity)
            } else {
                WontonNotice(icon: .warning, title: "这个码过期了。") {
                    Button("换一个码", action: renew)
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        }
        Text("在新设备上打开云吞 App 扫这个码，它就成为你的设备。码只能用一次。")
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text("不能扫码时粘贴这段文字")
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

/// 用 Core Image 画二维码：每个模块放大成整数个像素，缩放时不会糊。
private enum QRImage {
    static func make(_ text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        // 深色页面上二维码四周也要有白边，否则扫不出来。
        guard let code = filter.outputImage else { return nil }
        let quiet = code.extent.insetBy(dx: -2, dy: -2)
        let framed = code.composited(over: CIImage(color: .white)).cropped(to: quiet)
        let output = framed.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: image)
    }
}
