import AuthenticationServices
import Observation
import UIKit
import WontonClient

/// Matrix 登录的网页登录面板（`ASWebAuthenticationSession`）。和 Safari 共用登录状态；回到回调地址时由系统面板截住、
/// 交回这里，不经 App 的 URL scheme。回调原样交给核心核对，壳不解析授权码和 state。
@MainActor
@Observable
final class MatrixLoginBrowser: NSObject {
    /// 系统没打开面板（比如找不到窗口），页面提示并留“打开浏览器”。
    private(set) var failedToOpen = false
    private var session: ASWebAuthenticationSession?
    /// 当前面板的记号：面板的结果回来时核对，已经收起或换过的面板的结果不算数，免得把核心里新的一步取消掉。
    private var token: UUID?
    private var loginId = ""
    private var send: (Action) -> Void = { _ in }

    var isOpen: Bool { session != nil }

    func open(url: String, redirectURI: String, loginId: String, send: @escaping (Action) -> Void) {
        close()
        guard let authorization = URL(string: url), let scheme = URL(string: redirectURI)?.scheme else {
            failedToOpen = true
            return
        }
        let token = UUID()
        // 系统在别的线程上调它：闭包要显式不隔离，否则 Swift 6 推断成主线程隔离，在别的线程上被调用时直接崩溃。
        let finish: @Sendable (URL?, (any Error)?) -> Void = { [weak self] callback, error in
            let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
            Task { @MainActor in self?.finish(token: token, callback: callback, cancelled: cancelled) }
        }
        let session: ASWebAuthenticationSession
        if #available(iOS 17.4, *) {
            session = ASWebAuthenticationSession(
                url: authorization, callback: .customScheme(scheme), completionHandler: finish
            )
        } else {
            session = ASWebAuthenticationSession(
                url: authorization, callbackURLScheme: scheme, completionHandler: finish
            )
        }
        session.presentationContextProvider = self
        self.session = session
        self.token = token
        self.loginId = loginId
        self.send = send
        failedToOpen = !session.start()
        if failedToOpen {
            self.session = nil
            self.token = nil
        }
    }

    private func finish(token: UUID, callback: URL?, cancelled: Bool) {
        guard self.token == token else { return }
        session = nil
        self.token = nil
        if let callback {
            let returned = MatrixOidcCallback(loginId: loginId, callbackUrl: callback.absoluteString)
            send(.home(action: .completeMatrixOidc(callback: returned)))
        } else if cancelled {
            send(.home(action: .cancelMatrixLogin(loginId: loginId)))
        } else {
            failedToOpen = true
        }
    }

    /// 收起面板，不通知核心：调用方要么已经离开这一步，要么自己发取消。
    func close() {
        let open = session
        session = nil
        token = nil
        open?.cancel()
    }
}

extension MatrixLoginBrowser: ASWebAuthenticationPresentationContextProviding {
    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            return windows.first(where: \.isKeyWindow) ?? windows.first ?? ASPresentationAnchor()
        }
    }
}
