import AVFoundation
import SwiftUI
import VisionKit
import WontonClient

/// 扫码配对：扫家里服务器状态页上（或服务器上 `wonton-core pair` 打出）的二维码，也可以粘贴码的文字。
/// 进展和结果都来自核心的 `HomeViewModel.pairing`；配上后调用 `onPaired`。
struct PairingView: View {
    @Bindable var model: HomeModel
    var onBack: (() -> Void)?
    var onPaired: () -> Void = {}

    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @State private var camera = CameraAccess.current
    @State private var scannerProblem: String?
    @State private var deviceName = UIDevice.current.name
    /// 核心当场拒绝的原因，比如扫到的不是配对码。
    @State private var refusal: Problem?
    /// 最近一次出示的码。扫到同一个码不再自动出示，失败后由“再试一次”重来。
    @State private var lastCode: String?
    @FocusState private var isEditingCode: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if hasFailedPairing, let pairing = model.viewModel.pairing {
                    PairingProgress(
                        status: pairing,
                        retry: lastCode.map { code in { pair(code) } },
                        back: leaveFailure
                    )
                } else {
                    pairingForm
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Color(colors.canvas))
        .safeAreaInset(edge: .top, spacing: 0) {
            if model.servers.adding {
                HStack {
                    Button(.commonCancel) {
                        if hasFailedPairing { leaveFailure() } else { model.send(.cancelAddServer) }
                    }
                        .frame(minWidth: WontonTheme.size.target, minHeight: WontonTheme.size.target)
                    Spacer()
                    Text(.serversAddTitle)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .accessibilityAddTraits(.isHeader)
                    Spacer()
                    Color.clear.frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .frame(minHeight: WontonTheme.size.topBar)
                .background(.ultraThinMaterial)
            }
        }
        .task { await askForCamera() }
        .onDisappear {
            if hasFailedPairing { model.send(.dismissPairingFailure) }
        }
        .onChange(of: model.pairingCode) { _, _ in refusal = nil }
        .onChange(of: scenePhase) { _, phase in
            // 可能刚从系统设置里改了相机权限回来。
            if phase == .active { camera = .current }
        }
        .onChange(of: model.viewModel.pairing) { old, new in
            if !model.servers.adding, old == .inProgress, new == nil { onPaired() }
        }
    }

    private var pairingForm: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                scanner
                Text(.pairingInstructions)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            VStack(spacing: WontonTheme.spacing.s8) {
                TextField(String(localized: .pairingPastePlaceholder), text: $model.pairingCode, axis: .vertical)
                    .accessibilityIdentifier("pairing-code")
                    .textFieldStyle(.plain)
                    .font(model.pairingCode.isEmpty ? nil : .caption.monospaced())
                    .focused($isEditingCode)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(1...5)
                    .wontonField()
                if let domain = pairingDomain {
                    Text(domain)
                        .wontonText(WontonTheme.typography.body)
                        .textSelection(.enabled)
                }
                Button {
                    pair(model.pairingCode)
                } label: {
                    Text(.pairingSubmit).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .accessibilityIdentifier("pairing-submit")
                .disabled(model.pairingCode.isEmpty || isPairing)
            }
            if let refusal {
                ProblemNotice(problem: refusal, icon: .error)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("pairing-refusal")
            }
            if let pairing = model.viewModel.pairing {
                PairingProgress(status: pairing, retry: nil, back: leaveFailure)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("pairing-progress")
            }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Text(.pairingDeviceName)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .accessibilityHidden(true)
                TextField(String(localized: .pairingDeviceName), text: $deviceName)
                    .accessibilityIdentifier("pairing-device-name")
                    .textFieldStyle(.plain)
                    .wontonField()
            }
        }
    }

    @ViewBuilder private var scanner: some View {
        switch camera {
        case .authorized:
            if let scannerProblem {
                WontonNotice(icon: .cameraOff, title: scannerProblem) {
                    Button(.pairingReopenCamera) { self.scannerProblem = nil }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            } else {
                QRScannerView(onCode: scanned) { scannerProblem = $0 }
                    .frame(height: 280)
                    .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.card))
            }
        case .notDetermined:
            HStack(spacing: WontonTheme.spacing.s12) {
                ProgressView()
                Text(.cameraWaiting).wontonText(WontonTheme.typography.body)
            }
            .foregroundStyle(Color(colors.textPrimary))
        case .denied:
            WontonNotice(icon: .cameraOff, title: String(localized: .pairingCameraDenied)) {
                Button(.cameraSettings) {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        case .unsupported:
            WontonNotice(icon: .cameraOff, title: String(localized: .qrScannerUnsupportedPaste), isError: false)
        }
    }

    private var pairingDomain: String? {
        URLComponents(string: model.pairingCode)?.queryItems?.last(where: { $0.name == "domain" })?.value
    }

    private var isPairing: Bool {
        model.viewModel.pairing == .inProgress
    }

    private var hasFailedPairing: Bool {
        if case .failed? = model.viewModel.pairing { return true }
        return false
    }

    private func leaveFailure() {
        guard hasFailedPairing else { return }
        refusal = nil
        model.send(.dismissPairingFailure)
        if model.servers.adding {
            model.send(.cancelAddServer)
        } else {
            onBack?()
        }
    }

    private func scanned(_ code: String) {
        guard !hasFailedPairing else { return }
        if code != lastCode { pair(code) }
    }

    private func pair(_ code: String) {
        guard !isPairing else { return }
        // 先收起键盘，避免挡住进展或失败后的操作。
        isEditingCode = false
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
enum CameraAccess {
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
    let back: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch status {
        case .inProgress:
            HStack(spacing: WontonTheme.spacing.s12) {
                ProgressView()
                Text(.pairingInProgress).wontonText(WontonTheme.typography.body)
            }
            .foregroundStyle(Color(colors.textPrimary))
        case let .failed(reason, message):
            WontonNotice(
                icon: .warning,
                title: Self.title(reason),
                lines: Self.hint(reason).map { [$0] } ?? []
            ) {
                if let retry, Self.canRetrySameCode(reason) {
                    Button(action: retry) {
                        Text(.pairingRetrySameCode).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle())
                }
                Button(action: back) {
                    Text(.pairingBack).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                if let detail = Self.detail(reason, message) {
                    ProblemDetail(text: detail, minimumTargetHeight: WontonTheme.size.target)
                }
            }
        }
    }

    private static func title(_ reason: PairingFailure) -> String {
        switch reason {
        case .rejected: String(localized: .pairingFailureRejected)
        case .expired: String(localized: .pairingFailureExpired)
        case .closed: String(localized: .pairingFailureClosed)
        case .transportUnavailable: String(localized: .pairingFailureTransport)
        case .upstream: String(localized: .pairingFailureUpstream)
        case .protocol: String(localized: .pairingFailureProtocol)
        }
    }

    /// 码过期、不认识、已停用这几类不再重复服务器的说明；本机失败补一句下一步。
    private static func hint(_ reason: PairingFailure) -> String? {
        switch reason {
        case .rejected, .expired: String(localized: .pairingNextStepNewCode)
        case .closed: String(localized: .pairingNextStepClosed)
        case .transportUnavailable: String(localized: .pairingHintTransport)
        case .protocol: String(localized: .pairingHintProtocol)
        case .upstream: nil
        }
    }

    /// 核心按 App 语言写的诊断收进详情；码无效的几类已说明下一步，不再重复。
    private static func detail(_ reason: PairingFailure, _ message: String) -> String? {
        switch reason {
        case .rejected, .expired, .closed: nil
        case .transportUnavailable, .upstream, .protocol: message
        }
    }

    /// 连不上和服务器出错可能是暂时的，同一个码可以再试；其余原因要换码或者更新。
    private static func canRetrySameCode(_ reason: PairingFailure) -> Bool {
        reason == .transportUnavailable || reason == .upstream
    }
}
