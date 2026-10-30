import AVFoundation
import SwiftUI
import VisionKit
import WontonClient

/// 扫码配对：扫家里服务器状态页上（或服务器上 `wonton-core pair` 打出）的二维码，也可以粘贴码的文字。
/// 进展和结果都来自核心的 `HomeViewModel.pairing`；配上后调用 `onPaired`。
struct PairingView: View {
    @Bindable var model: HomeModel
    var onPaired: () -> Void = {}

    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @State private var camera = CameraAccess.current
    @State private var scannerProblem: String?
    @State private var deviceName = UIDevice.current.name
    @State private var pastedCode = ""
    /// 核心当场拒绝的原因，比如扫到的不是配对码。
    @State private var refusal: String?
    /// 最近一次出示的码。扫到同一个码不再自动出示，失败后由“再试一次”重来。
    @State private var lastCode: String?

    var body: some View {
        Form {
            Section {
                scanner
            } footer: {
                Text("在家里的局域网里，用浏览器打开服务器的 IP 地址（装好时终端里给出），页面上有配对二维码。")
            }
            Section("或者粘贴配对码") {
                TextField("wonton://pair?…", text: $pastedCode, axis: .vertical)
                    .font(.caption.monospaced())
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(2...5)
                Button("配对") { pair(pastedCode) }
                    .disabled(pastedCode.isEmpty || isPairing)
            }
            Section("这台设备的名字") {
                TextField("这台设备的名字", text: $deviceName)
            }
            if let refusal {
                Section {
                    Label(refusal, systemImage: "xmark.octagon").foregroundStyle(.red)
                }
            }
            if let pairing = model.viewModel.pairing {
                Section { PairingProgress(status: pairing, retry: lastCode.map { code in { pair(code) } }) }
            }
        }
        .task { await askForCamera() }
        .onChange(of: scenePhase) { _, phase in
            // 可能刚从系统设置里改了相机权限回来。
            if phase == .active { camera = .current }
        }
        .onChange(of: model.viewModel.pairing) { old, new in
            if old == .inProgress, new == nil { onPaired() }
        }
    }

    @ViewBuilder private var scanner: some View {
        switch camera {
        case .authorized:
            if let scannerProblem {
                VStack(alignment: .leading, spacing: 8) {
                    Label(scannerProblem, systemImage: "camera.badge.ellipsis")
                    Button("重新打开相机") { self.scannerProblem = nil }
                }
            } else {
                QRScannerView(onCode: scanned) { scannerProblem = $0 }
                    .frame(height: 280)
                    .listRowInsets(EdgeInsets())
            }
        case .notDetermined:
            ProgressView("等待相机权限…")
        case .denied:
            VStack(alignment: .leading, spacing: 8) {
                Text("云吞没有相机权限，扫不了码。可以在系统设置里打开，或者粘贴配对码。")
                Button("打开系统设置") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
            }
        case .unsupported:
            Text("这台设备不能扫码，请粘贴配对码。")
        }
    }

    private var isPairing: Bool {
        model.viewModel.pairing == .inProgress
    }

    private func scanned(_ code: String) {
        if code != lastCode { pair(code) }
    }

    private func pair(_ code: String) {
        guard !isPairing else { return }
        lastCode = code
        refusal = model.pair(code: code, deviceName: deviceName)
    }

    private func askForCamera() async {
        guard camera == .notDetermined else { return }
        _ = await AVCaptureDevice.requestAccess(for: .video)
        camera = .current
    }
}

/// 能不能用相机扫码。
private enum CameraAccess {
    case unsupported
    case notDetermined
    case denied
    case authorized

    @MainActor static var current: CameraAccess {
        guard DataScannerViewController.isSupported else { return .unsupported }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .notDetermined:
            return .notDetermined
        case .authorized:
            return .authorized
        case .denied, .restricted:
            return .denied
        @unknown default:
            return .denied
        }
    }
}

/// 配对的进展；失败时按原因说下一步怎么做。
private struct PairingProgress: View {
    let status: PairingStatus
    /// 用最近一次的码再试一次；只对可能是暂时的失败显示。
    let retry: (() -> Void)?

    var body: some View {
        switch status {
        case .inProgress:
            HStack(spacing: 12) {
                ProgressView()
                Text("正在配对…")
            }
        case let .failed(reason, message):
            VStack(alignment: .leading, spacing: 6) {
                Label(Self.title(reason), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                Text(message)
                if let hint = Self.hint(reason) {
                    Text(hint).foregroundStyle(.secondary)
                }
                if let retry, Self.canRetrySameCode(reason) {
                    Button("用这个码再试一次", action: retry)
                }
            }
            .font(.callout)
        }
    }

    private static func title(_ reason: PairingFailure) -> String {
        switch reason {
        case .rejected: "服务器不认这个配对码"
        case .expired: "配对码过期了"
        case .closed: "这个配对码已经停用"
        case .transportUnavailable: "连不上家里的服务器"
        case .upstream: "家里的服务器出错了"
        case .protocol: "App 和服务器的版本对不上"
        }
    }

    /// 服务器拒绝时，它的说明里已经写了下一步；本机这一侧的失败补一句。
    private static func hint(_ reason: PairingFailure) -> String? {
        switch reason {
        case .transportUnavailable: "请确认手机和服务器在同一个局域网里，或者服务器连得上互联网，然后再扫一次。"
        case .protocol: "请把 App 和服务器都更新到最新版本。"
        case .rejected, .expired, .closed, .upstream: nil
        }
    }

    /// 连不上和服务器出错可能是暂时的，同一个码可以再试；其余原因要换码或者更新。
    private static func canRetrySameCode(_ reason: PairingFailure) -> Bool {
        reason == .transportUnavailable || reason == .upstream
    }
}
