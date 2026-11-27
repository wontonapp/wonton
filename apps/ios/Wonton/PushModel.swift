import Foundation
import Observation
import UIKit
import UserNotifications
import WontonClient

/// 推送（ADR-0003 的“设计：加密推送”）：申请通知权限、拿 APNs 令牌，向推送中继要推送地址，把地址和
/// 这台设备的推送公钥登记给核心（和 Android 登记 UnifiedPush 地址走同一个 `SetPushEndpoint`）。进展和失败
/// 原因在设置的通知页如实显示，不假装已开启。
///
/// 中继为我们运营，地址是 App 构建配置（Info.plist 的 `WontonPushRelay`），不是核心下发；没有配置，或者
/// 没给通知权限、APNs 注册失败时，核心收到 `PushUnavailable`，设置页按原因类别写文字。
@MainActor
@Observable
final class PushModel: NSObject, UNUserNotificationCenterDelegate {
    /// UIApplication 只有一个 delegate（`FullscreenOrientation`），注册结果经它转过来。
    nonisolated(unsafe) static weak var shared: PushModel?

    /// 壳这一段收不了推送的原因；行文字按类别写，原始错误进详情（AGENTS.md“失败即停”）。
    enum Unavailability {
        /// 没给通知权限：系统不会送通知。
        case permission
        /// 读不到共享钥匙串里的推送密钥（未签名的构建、钥匙串访问组没生效）。
        case keychain(String)
        /// 系统注册远程通知失败（APNs 不可用）。
        case apns(String)
        /// 构建配置里没有推送中继。
        case noRelay
        /// 中继暂时连不上。
        case relayUnreachable(String)
        /// 核心当场拒绝了登记（地址不合法、设置写不进去等）；登记过程中的网络失败由核心记成 `Failed`。
        case core(String)

        var line: String {
            switch self {
            case .permission:
                String(localized: .Settings.pushNoPermission)
            case .keychain:
                String(localized: .Settings.pushKeysUnavailable)
            case .noRelay, .apns:
                String(localized: .Settings.pushNoService)
            case .relayUnreachable:
                String(localized: .Settings.pushRelayUnreachable)
            case .core:
                String(localized: .Settings.pushRegisterRejected)
            }
        }

        /// 发给核心的一句话（`PushUnavailable.message`，设置页的详情里照实显示）。
        var message: String {
            switch self {
            case .noRelay:
                String(localized: .Settings.pushNoRelayDetail)
            case let .apns(error), let .relayUnreachable(error), let .core(error),
                let .keychain(error):
                error
            case .permission:
                line
            }
        }
    }

    let core: ClientCore
    /// 壳这一段失败的原因；登记成功或开始登记时清掉。设置页用它写主行文字。
    private(set) var unavailability: Unavailability?
    private var running = false
    private var token: Data?
    private var keys: PushKeys.Registration?
    /// 密钥读不到的原因，进设置页的详情；`register` 半路被叫起来时也有话可说。
    private var keychainError: String?

    init(core: ClientCore) {
        self.core = core
        super.init()
        // 委托要赶在 App 跑完启动前设好：前台呈现和点开跳转都经它。
        UNUserNotificationCenter.current().delegate = self
        Self.shared = self
    }

    /// 在主界面的 `.task` 里调用；任务被取消就停下，下次再跑（幂等）。
    func start() async {
        guard !running else { return }
        running = true
        defer { running = false }
        // 密钥先备好：不依赖配对和权限，通知扩展只认它。失败要说出口：模拟器验证抓不到密钥时
        // 至少知道是哪一步坏了（未签名的构建读不了共享钥匙串，就是在这暴露的）。
        do {
            keys = try PushKeys.registration()
            keychainError = nil
        } catch {
            // Release 下没有控制台：原因进设置页，核心也别停在“未注册”还一个字不说。
            keys = nil
            keychainError = String(describing: error)
            setUnavailable(.keychain(keychainError ?? String(describing: error)))
            #if DEBUG
            if ProcessInfo.processInfo.environment["WONTON_DEV_PUSH_KEYS"] != nil {
                print("PUSH_KEYS_ERR \(error)")
                fflush(stdout)
            }
            #endif
        }
        #if DEBUG
        devPrintKeys()
        #endif
        // 权限框不能盖住配对页：等配好对再要（同 Android）。
        var paired = false
        for await home in core.subscribeHome().updates() where home.status != .notConfigured {
            paired = true
            break
        }
        guard paired else { return }
        // iOS 只有系统推送：收新消息的方式存成本机的“壳给推送地址”，复用 UnifiedPush 取值
        // （ADR-0003：iOS 不显示这个选项；核心拒绝保持连接下登记地址）。
        do {
            try core.dispatch(action: .push(action: .setPushMethod(method: .unifiedPush)))
        } catch {
            setUnavailable(.core(String(describing: error)))
            return
        }
        // UI 冒烟和走查自己决定什么时候问：启动就弹的权限框会吃掉第一个点按。
        #if DEBUG
        if ProcessInfo.processInfo.environment["WONTON_DEV_SKIP_PUSH_AUTH"] != nil { return }
        #endif
        // 通知权限：系统设置里拒绝过就不再弹框，这里如实记录。
        let granted = (try? await UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound])) ?? false
        guard granted else {
            setUnavailable(.permission)
            return
        }
        UIApplication.shared.registerForRemoteNotifications()
        // 令牌常在启动时就已经拿到（didRegister 回调），直接登记。
        if let token {
            await register(token: token)
        }
    }

    /// 回到前台：壳这一段失败过（权限刚给、中继连上了）就重试；登记过就不动。
    func foregrounded() {
        guard unavailability != nil, !running else { return }
        Task { await start() }
    }

    /// `FullscreenOrientation` 转来的注册结果。
    func registered(token: Data) {
        self.token = token
        guard keys != nil, !running else { return }
        Task { await register(token: token) }
    }

    func registrationFailed(_ error: Error) {
        // 没登记成功过，核心那边不用动；原因写进设置页。
        setUnavailable(.apns(String(describing: error)))
    }

    /// 向中继要推送地址，把地址和公钥登记给核心。
    private func register(token: Data) async {
        guard let keys else {
            setUnavailable(.keychain(keychainError ?? String(localized: .Settings.pushKeysUnavailable)))
            return
        }
        guard let base = Self.relayURL else {
            setUnavailable(.noRelay)
            return
        }
        let endpoint: String
        do {
            endpoint = try await Self.requestEndpoint(from: base, token: token)
        } catch {
            setUnavailable(.relayUnreachable(String(describing: error)))
            return
        }
        do {
            try core.dispatch(action: .push(action: .setPushEndpoint(
                endpoint: PushEndpoint(url: endpoint, p256dh: keys.p256dh, auth: keys.auth)
            )))
        } catch {
            setUnavailable(.core(String(describing: error)))
            return
        }
        unavailability = nil
    }

    private func setUnavailable(_ reason: Unavailability) {
        unavailability = reason
        try? core.dispatch(action: .push(action: .pushUnavailable(message: reason.message)))
    }

    /// 中继的地址，构建配置；空着就是推送还没开通。
    private static var relayURL: URL? {
        guard
            let text = Bundle.main.object(forInfoDictionaryKey: "WontonPushRelay") as? String,
            !text.trimmingCharacters(in: .whitespaces).isEmpty
        else { return nil }
        return URL(string: text)
    }

    /// 中继为这个 APNs 令牌开的推送地址：`POST <base>/v1/apns {"token": <hex>}` → `{"endpoint": "https://…"}`。
    /// 这是和中继之间的约定（ADR-0003 的“渠道”：中继为 App 的 APNs 令牌开一个 Web Push 推送资源）。
    private static func requestEndpoint(from base: URL, token: Data) async throws -> String {
        var request = URLRequest(url: base.appendingPathComponent("v1/apns"))
        request.httpMethod = "POST"
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "token": token.map { String(format: "%02x", $0) }.joined()
        ])
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200,
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let endpoint = json["endpoint"] as? String, URL(string: endpoint) != nil
        else {
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            throw CocoaError(
                .featureUnsupported,
                userInfo: [NSLocalizedDescriptionKey: "relay status \(status)"]
            )
        }
        return endpoint
    }

    /// App 在前台不弹推送：消息从同步来，界面已经显示。
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([])
    }

    /// 点开通知：扩展解开的 payload 里带着服务器和房间，跳转交给导航（和 Android 点通知同一个去向）。
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) {
        let userInfo = response.notification.request.content.userInfo
        guard let roomId = userInfo["room_id"] as? String else { return }
        let server = userInfo["server"] as? String
        Task { @MainActor in
            guard let navigation = try? AppNavigation.requireShared() else { return }
            navigation.openPushRoom(server: server, roomId: roomId)
        }
    }

    #if DEBUG
        /// 开发时 `WONTON_DEV_PUSH_KEYS=1`：把登记用的公钥和认证密钥打到标准输出，模拟器上的推送
        /// 验证要用它加密样例负载（simctl push 不经中继，见任务卡的验证一节）。认证密钥能伪造发给
        /// 这台设备的推送，只在 Debug 构建和显式开了环境变量时打。
        private func devPrintKeys() {
            guard
                ProcessInfo.processInfo.environment["WONTON_DEV_PUSH_KEYS"] != nil,
                let keys
            else { return }
            print("PUSH_KEYS p256dh=\(keys.p256dh) auth=\(keys.auth)")
            // 模拟器验证里 simctl terminate 是 SIGKILL，缓冲不冲；不冲的话密钥到不了抓取脚本。
            fflush(stdout)
        }
    #endif
}
