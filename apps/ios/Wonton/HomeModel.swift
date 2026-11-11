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
    private(set) var connectionRecovered = false
    private var recoveryFeedback: Task<Void, Never>?
    private var connectionInterrupted = false
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
            if viewModel.serverId != self.viewModel.serverId {
                connectionInterrupted = false
                recoveryFeedback?.cancel()
                connectionRecovered = false
            } else if viewModel.status != .connected {
                recoveryFeedback?.cancel()
                connectionRecovered = false
                connectionInterrupted = connectionInterrupted || viewModel.notice != nil
            } else if connectionInterrupted {
                connectionInterrupted = false
                connectionRecovered = true
                recoveryFeedback = Task {
                    do {
                        try await Task.sleep(for: .seconds(4))
                        connectionRecovered = false
                    } catch {}
                }
            }
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

    func prefillPairing(from url: URL) -> Bool {
        guard url.scheme == "wonton", url.host == "pair" else { return false }
        let current = core.home()
        guard current.status == .notConfigured, !core.servers().adding,
              current.pairing != .inProgress else { return false }
        pairingCode = url.absoluteString
        if case .failed? = current.pairing { send(.dismissPairingFailure) }
        return true
    }

    /// 扫到的不是配对码时核心当场拒绝，原因返回给配对页就地显示，不弹窗；之后的进展在 `viewModel.pairing` 里。
    func pair(code: String, deviceName: String) -> Problem? {
        do {
            try core.dispatch(action: .pairHome(code: code, deviceName: deviceName))
            return nil
        } catch {
            return error.problem
        }
    }
}
