import CoreSpotlight
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
            case let .ready(chat, home, plugins, appearance, navigation, systemEntry, lock):
                WontonThemeRoot(appearance: appearance.viewModel) {
                    // 锁页盖在整棵树上：深链、设置页和插件网页都挡得住（ADR-0102）。
                    AppLockGate(lock: lock) {
                        #if DEBUG
                        if AgentRunPreview.active {
                            AgentRunPreview.root
                        } else if QualityPreviews.active {
                            QualityPreviews.root
                        } else {
                            MainView(
                                chat: chat, home: home, plugins: plugins, appearance: appearance,
                                lock: lock, navigation: navigation, devWebURL: Launch.devWebURL
                            )
                        }
                        #else
                        MainView(
                            chat: chat, home: home, plugins: plugins, appearance: appearance,
                            lock: lock, navigation: navigation, devWebURL: Launch.devWebURL
                        )
                        #endif
                    }
                }
                .environmentObject(orientation)
                .task { await appearance.observe() }
                .task { await lock.observe() }
                .onContinueUserActivity(CSSearchableItemActionType) { activity in
                    guard let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String,
                          let url = URL(string: identifier) else { return }
                    navigation.receive(url)
                }
                .onChange(of: scenePhase) { old, phase in
                    // 挂起期间服务端可能已经放弃了连接、网络也可能变了，让核心检查并重连。离开后台就发，不等
                    // .active（实测晚约 0.4 秒）：核心要赶在网页恢复后的第一个请求之前开始检查。
                    if old == .background, phase != .background { home.send(.home(action: .appForegrounded)) }
                    if phase == .background { home.send(.home(action: .appBackgrounded)) }
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
    case ready(
        ChatModel, HomeModel, PluginsModel, AppearanceModel, AppNavigation, SystemEntryModel,
        AppLockModel)
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
        let directory: URL
        do {
            directory = try dataDirectory()
            #if DEBUG
            // UI 测试的假数据入口（见 crates/wonton-client/src/client/ui_fake.rs）：不进主界面以外的路径。
            if ProcessInfo.processInfo.environment["WONTON_UI_FAKE"] != nil {
                core = try ClientCore.newUiFake(dataDir: directory.path(percentEncoded: false))
            } else {
                core = try ClientCore(dataDir: directory.path(percentEncoded: false))
            }
            #else
            core = try ClientCore(dataDir: directory.path(percentEncoded: false))
            #endif
        } catch {
            return .failed(String(localized: .appCoreLaunchFailed(String(describing: error))))
        }
        // 核心在收到 `setLanguages` 之前按英文写“详情”。主界面 `.task` 要等第一帧之后才跑，配对页的
        // 第一帧和 DEBUG 的配对码那条都在它前面，所以在这里发。iOS 改 App 语言会重启 App，启动一次就够。
        do {
            try core.dispatch(action: .settings(action: .setLanguages(languages: AppLanguage.preferred)))
        } catch {
            return .failed(String(localized: .appCoreLaunchFailed(String(describing: error))))
        }
        // 赶在核心接着传上次没传完的文件之前。
        core.setFileOpener(opener: BookmarkFileOpener())
        #if DEBUG
        // 开发时由 `xcrun simctl launch` / `devicectl` 传入配对码（`dev.sh pair` 的输出），模拟器上不用扫码。
        if let code = ProcessInfo.processInfo.environment["WONTON_DEV_PAIRING_CODE"] {
            do {
                try core.dispatch(action: .home(action: .pairHome(code: code, deviceName: UIDevice.current.name)))
            } catch {
                return .failed(String(localized: .appDevCodeRejected(String(describing: error))))
            }
        }
        #endif
        let chat = ChatModel(core: core)
        let home = HomeModel(core: core)
        let plugins = PluginsModel(core: core)
        let navigation = AppNavigation(chat: chat, home: home, plugins: plugins)
        let systemEntry = SystemEntryModel(core: core, dataDirectory: directory)
        return .ready(
            chat, home, plugins, AppearanceModel(core: core), navigation, systemEntry,
            AppLockModel(core: core))
    }

    /// 核心的数据目录，存设备密钥和连接信息。
    private static func dataDirectory() throws -> URL {
        let base = try groupDirectory()
        #if DEBUG
        if let suffix = ProcessInfo.processInfo.environment["WONTON_DEV_DATA_SUFFIX"] {
            return base.deletingLastPathComponent().appending(path: "wonton-\(suffix)", directoryHint: .isDirectory)
        }
        #endif
        return base
    }

    /// 在 App Group 的容器里，分享扩展才放得进收件目录（ADR-0097）。真机上拿不到容器时如实启动失败，不退回 App 私有目录：
    /// 那样分享扩展写的东西 App 永远看不到。
    private static func groupDirectory() throws -> URL {
        do {
            return try AppGroup.dataDirectory()
        } catch {
            #if targetEnvironment(simulator)
            // 不签名的模拟器构建（CI 和脚本里的 `CODE_SIGNING_ALLOWED=NO`）没有 entitlements，系统不给组容器
            // （iOS 26.5 模拟器实测）。这种构建退回 App 私有目录，分享扩展同样拿不到容器、会如实报错；签了名的模拟器构建
            // 和真机都走组容器。
            if case .containerUnavailable = error {
                return try FileManager.default.url(
                    for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
                ).appending(path: "wonton", directoryHint: .isDirectory)
            }
            #endif
            throw error
        }
    }
}
