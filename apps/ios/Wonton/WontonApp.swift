import Foundation
import SwiftUI
import WontonClient

@main
struct WontonApp: App {
    @Environment(\.scenePhase) private var scenePhase
    /// 整个 App 只有一个核心实例，状态都在它里面。
    @State private var launch = Launch.start()
    @State private var tab = Launch.initialTab

    var body: some Scene {
        WindowGroup {
            switch launch {
            case let .ready(sessions, home):
                TabView(selection: $tab) {
                    SessionListView(model: sessions)
                        .tabItem { Label("会话", systemImage: "bubble.left.and.bubble.right") }
                        .tag(AppTab.sessions)
                    HomeView(model: home)
                        .tabItem { Label("家", systemImage: "house") }
                        .tag(AppTab.home)
                }
                .onChange(of: scenePhase) { old, phase in
                    // 挂起期间服务端可能已经放弃了连接、网络也可能变了，让核心检查并重连。离开后台就发，不等
                    // .active（实测晚约 0.4 秒）：核心要赶在网页恢复后的第一个请求之前开始检查。
                    if old == .background, phase != .background { home.send(.appForegrounded) }
                }
            case let .failed(reason):
                ContentUnavailableView("核心启动失败", systemImage: "exclamationmark.triangle",
                                       description: Text(reason))
            }
        }
    }
}

private enum AppTab {
    case sessions
    case home
}

/// 启动核心。核心失败时如实显示原因，不进入主界面。
@MainActor
private enum Launch {
    case ready(SessionListModel, HomeModel)
    case failed(String)

    /// 开发时 WONTON_DEV_OPEN_WEB=1 直接停在“家”，网页就绪后自动打开（见 HomeView）。
    static var initialTab: AppTab {
        #if DEBUG
        if ProcessInfo.processInfo.environment["WONTON_DEV_OPEN_WEB"] != nil { return .home }
        #endif
        return .sessions
    }

    static func start() -> Launch {
        #if DEBUG
        // WONTON_LOG 可以换过滤规则，例如真机测试时打开本地代理的 debug 日志。
        let filter = ProcessInfo.processInfo.environment["WONTON_LOG"] ?? "wonton_client=info,wonton_tunnel=info,warn"
        initLogging(filter: filter, file: nil)
        #endif
        do {
            let core = try ClientCore(dataDir: dataDirectory().path(percentEncoded: false))
            // 开发时由 `xcrun simctl launch` / `devicectl` 传入连接信息（dev.sh ticket 的输出）。
            if let ticket = ProcessInfo.processInfo.environment["WONTON_DEV_TICKET"] {
                try core.dispatch(action: .connectHome(ticket: ticket))
            }
            return .ready(SessionListModel(core: core), HomeModel(core: core))
        } catch {
            return .failed(String(describing: error))
        }
    }

    /// App 私有的数据目录，存设备密钥和连接信息。
    private static func dataDirectory() throws -> URL {
        let base = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        return base.appending(path: "wonton", directoryHint: .isDirectory)
    }
}
