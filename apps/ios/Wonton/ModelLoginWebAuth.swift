import AuthenticationServices
import Observation
import UIKit

/// 能回跳的模型登录（ADR-0129）：系统的登录面板（`ASWebAuthenticationSession`），和 Safari 共用登录状态。
/// 回到 `wonton://` 的回跳由面板截住交回这里，不经 App 的链接入口；回跳网址原样交给核心，壳不解析。
@MainActor
@Observable
final class ModelLoginWebAuth: NSObject {
    enum Result {
        case callback(URL)
        /// 人自己关了面板，不算出错。
        case cancelled
        case failed
    }

    /// 回跳地址 `wonton://model-login`（协议里的 `MODEL_LOGIN_REDIRECT_URI`）。只按 scheme 截的话，
    /// 期间别的 `wonton://` 链接也会被当回跳交回核心。
    private static let callbackScheme = "wonton"
    private static let callbackPrefix = "wonton://model-login"

    /// 系统没打开面板（比如找不到窗口）。
    private(set) var failedToOpen = false
    private var session: ASWebAuthenticationSession?
    /// 当前面板的记号：已经收起或换过的面板的结果不算数，免得把核心里新的一次登录取消掉。
    private var token: UUID?
    private var finished: ((Result) -> Void)?

    func open(url: URL, finished: @escaping (Result) -> Void) {
        close()
        let token = UUID()
        // 系统在别的线程上调它：闭包要显式不隔离，否则 Swift 6 推断成主线程隔离，在别的线程上被调用时直接崩溃。
        let completed: @Sendable (URL?, (any Error)?) -> Void = { [weak self] callback, error in
            let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
            Task { @MainActor in self?.finish(token: token, callback: callback, cancelled: cancelled) }
        }
        let session: ASWebAuthenticationSession
        if #available(iOS 17.4, *) {
            session = ASWebAuthenticationSession(
                url: url, callback: .customScheme(Self.callbackScheme), completionHandler: completed
            )
        } else {
            session = ASWebAuthenticationSession(
                url: url, callbackURLScheme: Self.callbackScheme, completionHandler: completed
            )
        }
        session.presentationContextProvider = self
        self.session = session
        self.token = token
        self.finished = finished
        failedToOpen = !session.start()
        if failedToOpen { finish(token: token, callback: nil, cancelled: false) }
    }

    private func finish(token: UUID, callback: URL?, cancelled: Bool) {
        guard self.token == token else { return }
        let finished = self.finished
        session = nil
        self.token = nil
        self.finished = nil
        if let callback {
            finished?(callback.absoluteString.hasPrefix(Self.callbackPrefix) ? .callback(callback) : .cancelled)
        } else {
            finished?(cancelled ? .cancelled : .failed)
        }
    }

    /// 收起面板，不回调：调用方要么已经离开这一步，要么自己发取消。
    func close() {
        let open = session
        session = nil
        token = nil
        finished = nil
        open?.cancel()
    }
}

extension ModelLoginWebAuth: ASWebAuthenticationPresentationContextProviding {
    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            return windows.first(where: \.isKeyWindow) ?? windows.first ?? ASPresentationAnchor()
        }
    }
}
