import Observation
import WontonClient

@MainActor
@Observable
final class SettingsModel {
    private(set) var viewModel: SettingsViewModel
    // 根视图持有路径，跨标签导航不依赖设置页是否已挂载。
    var path: [SettingsRoute] = []
    /// App 链接打开“其他服务器”并要求滚到“请求”；每次加一，页面按变化滚。
    private(set) var peersFocusRequests = 0
    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.settings()
    }

    #if DEBUG
        /// 截图测试拿固定 ViewModel 画界面，不订阅核心推送。
        init(core: ClientCore, viewModel: SettingsViewModel) {
            self.core = core
            self.viewModel = viewModel
        }
    #endif

    func observe() async {
        for await viewModel in core.subscribeSettings().updates() {
            self.viewModel = viewModel
        }
    }

    var hasServerUpgrade: Bool {
        viewModel.sections.contains { section in
            section.entries.contains { entry in
                if case let .server(_, _, upgrade) = entry { return upgrade }
                return false
            }
        }
    }

    func openServer(_ route: SettingsRoute) {
        path = route == .server ? [.server] : [.server, route]
    }

    func focusPeersRequests() {
        peersFocusRequests += 1
    }
}

enum SettingsRoute: Hashable {
    case server
    case matrixLogin
    case connectionDetails
    case invite
    case pairing
    case peers
    case friends
    case egress
    case registryMirror
    case coreUpgrade
    case appearance
    case chatBackup
    case myApps
    case notifications
    case appLock
    case about
}
