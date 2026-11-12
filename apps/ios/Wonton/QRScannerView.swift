import SwiftUI
import VisionKit

/// 相机取景并识别二维码，把码里的文字交给 `onCode`：码进入画面时报一次，移出再进来会再报。
///
/// 用 VisionKit 的 `DataScannerViewController`，iOS 17 支持的机型都能用；模拟器上 `isSupported` 为假。
/// 相机权限由调用方先取得。
struct QRScannerView: UIViewControllerRepresentable {
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
        guard !scanner.isScanning else { return }
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
            return String(localized: .qrScannerCameraRestricted)
        case .unsupported:
            return String(localized: .qrScannerUnsupported)
        default:
            return String(localized: .qrScannerStopped(error.localizedDescription))
        }
    }

    @MainActor
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        var parent: QRScannerView

        init(parent: QRScannerView) {
            self.parent = parent
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController,
            didAdd addedItems: [RecognizedItem],
            allItems: [RecognizedItem]
        ) {
            for case let .barcode(barcode) in addedItems {
                if let text = barcode.payloadStringValue { parent.onCode(text) }
            }
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController,
            becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable
        ) {
            parent.onUnavailable(QRScannerView.describe(error))
        }
    }
}
