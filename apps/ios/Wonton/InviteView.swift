import CoreImage.CIFilterBuiltins
import SwiftUI
import WontonClient

/// “添加设备”：向家里的服务器要一个一次性配对码，画成二维码给新设备上的云吞 App 扫。进展和码都来自核心的
/// `HomeViewModel.invite`。
struct InviteView: View {
    @Bindable var model: HomeModel
    let done: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                switch model.viewModel.invite {
                case nil, .requesting:
                    HStack(spacing: 12) {
                        ProgressView()
                        Text("正在向家里的服务器要配对码…")
                    }
                case let .ready(code, expiresAtMs):
                    InviteCode(code: code, expiresAt: Date(timeIntervalSince1970: Double(expiresAtMs) / 1000)) {
                        model.send(.inviteDevice)
                    }
                case let .failed(_, message):
                    Section {
                        Label(message, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
                        Button("再试一次") { model.send(.inviteDevice) }
                    } header: {
                        Text("没要到配对码")
                    }
                }
            }
            .navigationTitle("添加设备")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成", action: done)
                }
            }
        }
        .task { model.send(.inviteDevice) }
    }
}

/// 码的二维码、文字和剩下的有效期；过期后换一个。
private struct InviteCode: View {
    let code: String
    let expiresAt: Date
    let renew: () -> Void

    var body: some View {
        Section {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let remaining = Int(expiresAt.timeIntervalSince(context.date))
                if remaining > 0 {
                    VStack(spacing: 12) {
                        if let image = QRImage.make(code) {
                            Image(uiImage: image)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(maxWidth: 260)
                        }
                        Text("还有 \(remaining / 60):\(String(format: "%02d", remaining % 60)) 过期")
                            .font(.callout.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("这个码过期了。")
                        Button("换一个码", action: renew)
                    }
                }
            }
        } footer: {
            Text("在新设备上打开云吞 App 扫这个码，它就成为你的设备。码只能用一次。")
        }
        Section("不能扫码时粘贴这段文字") {
            Text(code)
                .font(.caption.monospaced())
                .textSelection(.enabled)
        }
    }
}

/// 用 Core Image 画二维码：每个模块放大成整数个像素，缩放时不会糊。
private enum QRImage {
    static func make(_ text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: image)
    }
}
