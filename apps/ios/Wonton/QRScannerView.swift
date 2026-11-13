import AVFoundation
import SwiftUI
import VisionKit

/// 相机取景并识别二维码，把码里的文字交给 `onCode`：码进入画面时报一次，移出再进来会再报。
///
/// 用 VisionKit 的 `DataScannerViewController`，iOS 17 支持的机型都能用；模拟器上 `isSupported` 为假。
/// 相机权限由调用方先取得。
struct QRScannerView: UIViewControllerRepresentable {
    var singleShot = false
    let onCode: @MainActor (String) -> Void
    /// 扫描用不了或中途停了，参数是给人看的原因。
    let onUnavailable: @MainActor (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isHighlightingEnabled: true
        )
        scanner.delegate = context.coordinator
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
        context.coordinator.parent = self
        guard !context.coordinator.hasDeliveredCode, !scanner.isScanning else { return }
        do {
            try scanner.startScanning()
        } catch {
            // 视图更新期间不能改 SwiftUI 的状态，放到下一轮。
            let reason = Self.describe(error)
            Task { @MainActor in onUnavailable(reason) }
        }
    }

    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
        scanner.stopScanning()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    fileprivate static func describe(_ error: Error) -> String {
        switch error as? DataScannerViewController.ScanningUnavailable {
        case .cameraRestricted:
            return String(localized: .Peers.qrScannerCameraRestricted)
        case .unsupported:
            return String(localized: .Peers.qrScannerUnsupported)
        default:
            return String(localized: .Peers.qrScannerStopped(error.localizedDescription))
        }
    }

    @MainActor
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        var parent: QRScannerView
        private(set) var hasDeliveredCode = false

        init(parent: QRScannerView) {
            self.parent = parent
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController,
            didAdd addedItems: [RecognizedItem],
            allItems: [RecognizedItem]
        ) {
            for case let .barcode(barcode) in addedItems {
                guard let text = barcode.payloadStringValue else { continue }
                deliver(text, stopScanning: dataScanner.stopScanning)
            }
        }

        func deliver(_ code: String, stopScanning: () -> Void) {
            guard !hasDeliveredCode else { return }
            if parent.singleShot {
                hasDeliveredCode = true
                stopScanning()
            }
            parent.onCode(code)
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController,
            becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable
        ) {
            parent.onUnavailable(QRScannerView.describe(error))
        }
    }
}

/// 扫码页上半部分：按相机权限显示取景框、等授权、去设置或“这台设备不能扫码”。配对和加好友共用，
/// 扫到的原文交给 `onCode`；粘贴框由各页自己放在下面。
struct QRCameraArea: View {
    /// 设备不能扫码时的提示：各页说该粘贴什么。
    let unsupported: LocalizedStringResource
    let onCode: @MainActor (String) -> Void
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @Environment(\.wontonColors) private var colors
    @State private var camera = CameraAccess.current
    @State private var scannerProblem: String?

    var body: some View {
        Group {
            switch camera {
            case .authorized:
                if let scannerProblem {
                    WontonNotice(icon: .cameraOff, title: scannerProblem) {
                        Button(.Peers3.peersCameraReopen) { self.scannerProblem = nil }
                            .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                } else {
                    QRScannerView(onCode: onCode) { scannerProblem = $0 }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.card))
                }
            case .notDetermined:
                HStack(spacing: WontonTheme.spacing.s12) {
                    ProgressView()
                    Text(.Peers3.peersCameraWaiting).wontonText(WontonTheme.typography.body)
                }
                .foregroundStyle(Color(colors.textPrimary))
            case .denied:
                WontonNotice(icon: .cameraOff, title: String(localized: .Peers1.peersCameraDenied)) {
                    Button(.Common.commonOpenSettings) {
                        if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            case .unsupported:
                WontonNotice(icon: .cameraOff, title: String(localized: unsupported), isError: false)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task { await askForCamera() }
        .onChange(of: scenePhase) { _, phase in
            // 可能刚从系统设置里改了相机权限回来。
            if phase == .active { camera = .current }
        }
    }

    private func askForCamera() async {
        guard camera == .notDetermined else { return }
        _ = await AVCaptureDevice.requestAccess(for: .video)
        camera = .current
    }
}
