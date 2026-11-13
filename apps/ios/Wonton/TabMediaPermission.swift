import AVFoundation
import Observation
import SwiftUI
import WebKit
import WontonClient

/// 网页要摄像头/麦克风的协调（ADR-0033）：壳的询问弹窗整份 App 一份（挂在 MainView），
/// 网页的请求在哪个会话里发起，回答就还到哪份。同一时刻只有一份在问、一份在系统这层，
/// 多出来的直接拒，不让网页干等。
@MainActor
@Observable
final class TabMediaRequests {
    static let shared = TabMediaRequests()

    /// 核心出了询问、等主人答的那份网页请求：`.allow`/`.deny` 是交核心记下的回答，
    /// `nil` 是没答成（页面先关了、换了一份请求），只拒网页的请求。
    private var promptRequest: ((TabMediaPermission?) -> Void)?
    /// 核心放行、在等系统运行时权限的那份网页请求。
    private var systemRequest: ((Bool) -> Void)?
    /// 系统层拒过：提示主人去设置。
    var systemDenied = false

    /// 核心出了询问：挂上等回答的那份；又来一份时旧的直接拒（同 Android）。
    func asked(for type: WKMediaCaptureType, answer: @escaping (Bool) -> Void) {
        promptRequest?(nil)
        promptRequest = { decision in
            if decision == .allow {
                Self.shared.systemStep(for: type, answer: answer)
            } else {
                answer(false)
            }
        }
    }

    /// 主人答了询问弹窗：决定已交核心记下，网页的请求按核心给的处置走。
    func answered(_ decision: TabMediaPermission?) {
        guard let request = promptRequest else { return }
        promptRequest = nil
        request(decision)
    }

    /// 询问被核心清了（页面关了）：没人可答的网页请求拒掉，别让网页干等。
    func cancelled() {
        answered(nil)
    }

    /// 核心放行后再拿系统运行时权限；系统层拒过的不再弹（状态不是未决定），
    /// 网页拿到拒绝，壳给一条带“去设置”的提示。
    func systemStep(for type: WKMediaCaptureType, answer: @escaping (Bool) -> Void) {
        if systemRequest != nil { answer(false); return }
        systemRequest = answer
        Task { @MainActor in
            let granted = await Self.systemPermission(for: type)
            self.systemRequest = nil
            if granted {
                answer(true)
            } else {
                answer(false)
                self.systemDenied = true
            }
        }
    }

    /// App 内的允许是 App 内的决定，系统权限每次都另查（ADR-0033）。
    private static func systemPermission(for type: WKMediaCaptureType) async -> Bool {
        switch type {
        case .camera:
            await granted(.video)
        case .microphone:
            await granted(.audio)
        case .cameraAndMicrophone:
            if await granted(.video) { await granted(.audio) } else { false }
        @unknown default:
            false
        }
    }

    private static func granted(_ media: AVMediaType) async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: media) {
        case .authorized:
            true
        case .notDetermined:
            await AVCaptureDevice.requestAccess(for: media)
        default:
            false
        }
    }
}

extension TabWebSession: WKUIDelegate {
    /// 网页要摄像头/麦克风（getUserMedia）：先问核心（按插件记决定），按回答放行或拒绝；
    /// 放行还要系统运行时权限。其他 UI 代理方法走 WebKit 的默认。
    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor initiatedByFrame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void
    ) {
        let camera = type == .camera || type == .cameraAndMicrophone
        let microphone = type == .microphone || type == .cameraAndMicrophone
        // 源从 WebView 的原生回调取，核心按页面自己的源核对；页面里嵌的其他源核心直接拒。
        // 核心问不出来（页面关了、源不符、查记录失败）就是拒：如实拒网页并留日志，不静默。
        let decision: TabMediaPermission?
        do {
            decision = try core.tabMediaPermissionRequest(
                page: placed, origin: Self.originString(initiatedByFrame.securityOrigin),
                camera: camera, microphone: microphone
            )
        } catch {
            decision = nil
            NSLog("[media] request failed origin=%@ error=%@",
                  Self.originString(initiatedByFrame.securityOrigin), String(describing: error))
        }
        let answer = { (allow: Bool) in decisionHandler(allow ? .grant : .deny) }
        switch decision {
        case .allow:
            TabMediaRequests.shared.systemStep(for: type, answer: answer)
        case .prompt:
            TabMediaRequests.shared.asked(for: type, answer: answer)
        default:
            answer(false)
        }
    }
}

/// 摄像头/麦克风的询问弹窗和系统层拒绝的提示。整个 App 挂一份：挂在各页的话，
/// 后台页的那份先弹，弹不出来，用户在别的页就看不到（同 ChatDialogs）。
struct TabMediaDialogs: ViewModifier {
    let model: PagesModel
    @State private var showsPrompt = false

    func body(content: Content) -> some View {
        content
            .wontonDialog(
                promptTitle,
                isPresented: $showsPrompt,
                actions: [
                    WontonDialogAction(
                        title: String(localized: .Tab2.tabMediaDeny),
                        perform: { answered(false) },
                        identifier: "tabMediaDeny"
                    ),
                    WontonDialogAction(
                        title: String(localized: .Tab0.tabMediaAllow),
                        kind: .primary,
                        perform: { answered(true) },
                        identifier: "tabMediaAllow"
                    ),
                ],

                onDismiss: { answered(false) }
            )
            .wontonDialog(
                String(localized: .Tab0.tabMediaSystemDeniedTitle),
                message: String(localized: .Tab0.tabMediaSystemDenied),
                isPresented: Binding(
                    get: { TabMediaRequests.shared.systemDenied },
                    set: { if !$0 { TabMediaRequests.shared.systemDenied = false } }
                ),
                actions: [
                    WontonDialogAction(
                        title: String(localized: .Tab0.tabMediaSystemSettings),
                        perform: {
                            TabMediaRequests.shared.systemDenied = false
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                UIApplication.shared.open(url)
                            }
                        }
                    ),
                ]
            )
            .onChange(of: model.viewModel.mediaPrompt, initial: true) { _, value in
                if value != nil {
                    showsPrompt = true
                } else if showsPrompt {
                    // 询问被核心清了（页面关了）：收起弹窗，网页的请求拒掉。
                    // 主人答过的走弹窗按钮，核心的决定已记下，弹窗先收起来，这里没人可清。
                    showsPrompt = false
                    TabMediaRequests.shared.cancelled()
                }
            }
    }

    private var promptTitle: String {
        guard let prompt = model.viewModel.mediaPrompt else { return "" }
        return switch (prompt.camera, prompt.microphone) {
        case (true, true): String(localized: .Tab2.tabMediaPromptBoth(prompt.pluginTitle))
        case (true, false): String(localized: .Tab1.tabMediaPromptCamera(prompt.pluginTitle))
        default: String(localized: .Tab0.tabMediaPromptMicrophone(prompt.pluginTitle))
        }
    }

    /// 主人答了：决定交核心记下（按插件记住），网页的请求按核心给的处置走。
    /// 记不下（询问已被清、写盘失败）就是没记住，如实告诉网页拒绝并留日志，不静默。
    private func answered(_ allow: Bool) {
        let decision: TabMediaPermission?
        do {
            decision = try model.core.tabMediaPermissionDecide(allow: allow)
            NSLog("[media] decide ok allow=%d", allow ? 1 : 0)
        } catch {
            decision = nil
            NSLog("[media] decide failed allow=%d error=%@", allow ? 1 : 0, String(describing: error))
        }
        TabMediaRequests.shared.answered(decision)
    }
}
