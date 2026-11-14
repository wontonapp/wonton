import SwiftUI
import WontonClient

/// 扫对方的码：扫到的字符串原样交给核心，成功才收起。码不能用时在粘贴框下面说，和设备配对页一样。
struct PeersScannerView: View {
    let model: PeersModel
    @Environment(\.dismiss) private var dismiss
    @State private var scanProblem: Problem?
    @State private var pastedCode = ""
    /// 码进画面报一次，移出再进来会再报；提交过就不再提交。
    @State private var submitted = false

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            QRCameraArea(unsupported: .Peers.qrScannerUnsupportedPaste, onCode: submit)
            pasteSection
        }
        .padding(WontonTheme.spacing.pageInset)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onChange(of: model.viewModel.state) { _, state in
            // 页面自己的首帧刷新也会走 Loading→Ready：没提交过就不收起。
            guard submitted else { return }
            switch state {
            case .loading:
                break
            case .ready:
                dismiss()
            case let .failed(error, message):
                // 码读不懂、已经作废：留在这一页改，交给用户换一个。
                switch error {
                case .codeInvalid, .codeRevoked:
                    scanProblem = Problem(title: scanTitle(error), detail: message)
                    submitted = false
                default:
                    dismiss()
                }
            }
        }
        .onChange(of: model.rejection != nil) { _, refused in
            // 核心当场拒绝（比如列表还没就绪）：弹窗在列表上说，这一页还能再提交。
            if refused { submitted = false }
        }
    }

    /// 扫不到时的退路：粘贴对方的配对码，提交方式和扫到的一样。
    private var pasteSection: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            TextField(String(localized: .Peers0.peersPasteCode), text: $pastedCode, axis: .vertical)
                .textFieldStyle(.plain)
                .font(pastedCode.isEmpty ? nil : .caption.monospaced())
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(1...5)
                .wontonField()
            if let scanProblem {
                ProblemNotice(problem: scanProblem)
            }
            Button {
                submit(pastedCode)
            } label: {
                Text(.Peers2.peersPasteSubmit).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(pastedCode.isEmpty || submitted)
        }
    }

    private func submit(_ code: String) {
        guard !submitted else { return }
        submitted = true
        scanProblem = nil
        // 成功后核心自己刷新列表，列表上多一行“等对方同意”；码不能用时留在这里说。
        model.send(.peers(action: .scanPeerCode(code: code)))
    }

    private func scanTitle(_ error: PeerError) -> String {
        switch error {
        case .codeRevoked: String(localized: .Peers1.peersErrorCodeRevoked)
        default: String(localized: .Peers0.peersErrorCodeInvalid)
        }
    }
}
