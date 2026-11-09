import Foundation
import SwiftUI
import UIKit
import WontonClient

@main
struct WontonApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @UIApplicationDelegateAdaptor(FullscreenOrientation.self) private var orientation
    /// 整个 App 只有一个核心实例，状态都在它里面。
    @State private var launch: Launch

    init() {
        let launch = Launch.start()
        _launch = State(initialValue: launch)
    }

    var body: some Scene {
        WindowGroup {
            switch launch {
            case let .ready(chat, home, plugins, appearance, navigation):
                WontonThemeRoot(appearance: appearance.viewModel) {
                    #if DEBUG
                    if QualityPreviews.active {
                        QualityPreviews.root
                    } else {
                        MainView(
                            chat: chat, home: home, plugins: plugins, appearance: appearance,
                            navigation: navigation, devWebURL: Launch.devWebURL
                        )
                    }
                    #else
                    MainView(
                        chat: chat, home: home, plugins: plugins, appearance: appearance,
                        navigation: navigation, devWebURL: Launch.devWebURL
                    )
                    #endif
                }
                .environmentObject(orientation)
                .task { await appearance.observe() }
                .onChange(of: scenePhase) { old, phase in
                    // 挂起期间服务端可能已经放弃了连接、网络也可能变了，让核心检查并重连。离开后台就发，不等
                    // .active（实测晚约 0.4 秒）：核心要赶在网页恢复后的第一个请求之前开始检查。
                    if old == .background, phase != .background { home.send(.appForegrounded) }
                    if phase == .background { home.send(.appBackgrounded) }
                }
            case let .failed(reason):
                WontonThemeRoot(appearance: defaultAppearance()) {
                    ContentUnavailableView(
                        String(localized: .appLaunchFailed), systemImage: "exclamationmark.triangle",
                        description: Text(reason)
                    )
                }
            }
        }
    }
}

/// 启动核心。核心失败时如实显示原因，不进入主界面。
@MainActor
private enum Launch {
    case ready(ChatModel, HomeModel, PluginsModel, AppearanceModel, AppNavigation)
    case failed(String)

    /// 开发时 `WONTON_DEV_OPEN_WEB=<插件网页地址>`：本地代理就绪后自动打开（见 HomeView）。
    static var devWebURL: URL? {
        #if DEBUG
        if let text = ProcessInfo.processInfo.environment["WONTON_DEV_OPEN_WEB"] { return URL(string: text) }
        #endif
        return nil
    }

    static func start() -> Launch {
        #if DEBUG
        // WONTON_LOG 可以换过滤规则，例如真机测试时打开本地代理的 debug 日志。
        let filter = ProcessInfo.processInfo.environment["WONTON_LOG"] ?? "wonton_client=info,wonton_tunnel=info,warn"
        initLogging(filter: filter, file: nil)
        #endif
        let core: ClientCore
        do {
            core = try ClientCore(dataDir: dataDirectory().path(percentEncoded: false))
        } catch {
            return .failed(String(localized: .appCoreLaunchFailed(String(describing: error))))
        }
        // 赶在核心接着传上次没传完的文件之前。
        core.setFileOpener(opener: BookmarkFileOpener())
        #if DEBUG
        // 开发时由 `xcrun simctl launch` / `devicectl` 传入配对码（`dev.sh pair` 的输出），模拟器上不用扫码。
        if let code = ProcessInfo.processInfo.environment["WONTON_DEV_PAIRING_CODE"] {
            do {
                try core.dispatch(action: .pairHome(code: code, deviceName: UIDevice.current.name))
            } catch {
                return .failed(String(localized: .appDevCodeRejected(String(describing: error))))
            }
        }
        #endif
        let chat = ChatModel(core: core)
        let home = HomeModel(core: core)
        let plugins = PluginsModel(core: core)
        let navigation = AppNavigation(chat: chat, home: home, plugins: plugins)
        return .ready(chat, home, plugins, AppearanceModel(core: core), navigation)
    }

    /// App 私有的数据目录，存设备密钥和连接信息。
    private static func dataDirectory() throws -> URL {
        let base = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        return base.appending(path: "wonton", directoryHint: .isDirectory)
    }
}
