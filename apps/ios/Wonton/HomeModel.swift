import Foundation
import Observation
import WontonClient

/// 持有核心推来的连接 ViewModel，把用户操作转发给核心。
@MainActor
@Observable
final class HomeModel {
    private(set) var viewModel: HomeViewModel
    private(set) var servers: ServersViewModel
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: Problem?
    /// 深链接和手动输入共用草稿；只有配对页确认后才交给核心。
    var pairingCode = ""

    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.home()
        servers = core.servers()
    }

    #if DEBUG
        /// 截图测试拿固定 ViewModel 画界面，不订阅核心推送。
        init(core: ClientCore, viewModel: HomeViewModel, servers: ServersViewModel) {
            self.core = core
            self.viewModel = viewModel
            self.servers = servers
        }
    #endif

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await viewModel in core.subscribeHome().updates() {
            if viewModel.serverId != nil { pairingCode = "" }
            self.viewModel = viewModel
        }
    }

    func observeServers() async {
        for await servers in core.subscribeServers().updates() {
            self.servers = servers
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    func dismissPairingFailure() {
        send(.home(action: .dismissPairingFailure))
        viewModel = core.home()
    }

    func cancelPairing() {
        send(.home(action: .cancelPairing))
        viewModel = core.home()
    }

    func prefillPairing(from url: URL) -> Bool {
        guard url.scheme == "wonton", url.host == "pair" else { return false }
        let current = core.home()
        guard current.status == .notConfigured, !core.servers().adding else { return false }
        // 配对正在进行时不接新的码。
        if case .inProgress = current.pairing { return false }
        pairingCode = url.absoluteString
        if case .failed? = current.pairing { send(.home(action: .dismissPairingFailure)) }
        return true
    }

    /// Matrix 登录的操作（ADR-0051）：核心当场拒绝的原因返回给登录页就地显示，之后的进展在 `servers.matrixLogin` 里。
    func matrixLogin(_ action: Action) -> Problem? {
        do {
            try core.dispatch(action: action)
            return nil
        } catch {
            return error.problem
        }
    }

    /// 系统把 Matrix 登录的回调地址交给了 App（网页登录面板以外的路径，比如在 Safari 里打开了回调）：有进行中的登录就
    /// 交给核心核对，是不是这次登录的由核心判断。地址按核心给的 `oidcRedirectUri` 认，壳不另写一份。
    func completeMatrixLogin(from url: URL) -> Bool {
        guard let login = core.servers().matrixLogin,
              let redirect = URLComponents(string: login.oidcRedirectUri),
              let received = URLComponents(url: url, resolvingAgainstBaseURL: false),
              received.scheme?.lowercased() == redirect.scheme?.lowercased(),
              // 从 App 外面打开的 `scheme:/path` 到这里成了 `scheme:///path`（空主机），是同一个地址。
              (received.host ?? "") == (redirect.host ?? ""), received.path == redirect.path
        else { return false }
        send(.home(action: .completeMatrixOidc(
            callback: MatrixOidcCallback(loginId: login.loginId, callbackUrl: url.absoluteString)
        )))
        return true
    }

    /// 扫到的不是配对码时核心当场拒绝，原因返回给配对页就地显示，不弹窗；之后的进展在 `viewModel.pairing` 里。
    func pair(code: String, deviceName: String) -> Problem? {
        defer { viewModel = core.home() }
        do {
            try core.dispatch(action: .home(action: .pairHome(code: code, deviceName: deviceName)))
            return nil
        } catch {
            let problem = error.problem
            return Problem(title: problem.title, detail: problem.nextStep ?? problem.detail)
        }
    }
}
