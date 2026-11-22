import SwiftUI
import UIKit
import UserNotifications

/// 云吞在系统里的通知权限（#677）。iOS 只给问过的 App 建云吞自己的通知页，没问过时去点，系统设置只停在首页或者
/// 云吞的普通应用页，所以入口要先把“问权限”走完，再去系统页。`provisional`、`ephemeral` 在本 App 用不到，和
/// `authorized` 一样算允许。
enum NotificationPermission: Equatable {
    case notAsked
    case allowed
    case denied

    init(_ status: UNAuthorizationStatus) {
        switch status {
        case .notDetermined: self = .notAsked
        case .denied: self = .denied
        case .authorized, .provisional, .ephemeral: self = .allowed
        @unknown default: self = .denied
        }
    }

    /// 系统里现在的状态。走回调版：async 版返回的 `UNNotificationSettings` 不是 Sendable，只有状态枚举
    /// （Int 的 NS_ENUM）穿过隔离边界。回调方法叫 `getNotificationSettings`，async 版才叫 `notificationSettings`。
    @MainActor
    static func current() async -> NotificationPermission {
        let status: UNAuthorizationStatus = await withCheckedContinuation { (done: CheckedContinuation<UNAuthorizationStatus, Never>) in
            UNUserNotificationCenter.current().getNotificationSettings(completionHandler: { done.resume(returning: $0.authorizationStatus) })
        }
        return NotificationPermission(status)
    }

    /// 弹一次系统的授权框；问过以后系统不再弹，立刻返回。
    @MainActor
    static func ask() async {
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
    }

    /// 点入口时先弹系统的授权框：只有还没问过的设备有这一步，允许、拒绝都照样去系统页。
    var asksBeforeOpening: Bool { self == .notAsked }

    /// 行右侧的系统状态。没问过和拒绝都写成“未允许”：两种情况下系统里都收不到通知，点一下分别会问、会去打开。
    var rowValue: String {
        switch self {
        case .notAsked, .denied: String(localized: .Settings.notificationDenied)
        case .allowed: String(localized: .Settings.notificationAllowed)
        }
    }

    /// 云吞自己的系统设置页：通知开关就在这页第一行，点进去是逐项开关。
    /// 用社区一直用的 `App-prefs:root=<bundle id>`（苹果没写进文档）；`openSettingsURLString` 的 `app-settings:` 在这台
    /// 模拟器上只把系统设置开到首页，`path=NOTIFICATIONS_ID` 也一样。解析不出时退回文档里的取值。
    static var systemSettingsURL: URL? {
        URL(string: "App-Prefs:root=\(Bundle.main.bundleIdentifier ?? "")")
            ?? URL(string: UIApplication.openSettingsURLString)
    }

    @MainActor
    static func openSystemSettings() async {
        if let target = systemSettingsURL { _ = await UIApplication.shared.open(target) }
    }
}
