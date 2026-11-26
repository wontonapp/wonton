import UIKit
import UserNotifications
import XCTest
@testable import Wonton

@MainActor
final class NotificationPermissionTests: XCTestCase {
    /// 只有没问过才先弹系统的授权框；问过以后再点就是直接去系统页，不再空弹一次。
    func testOnlyNotAskedAsksFirst() {
        XCTAssertTrue(NotificationPermission(.notDetermined).asksBeforeOpening)
        XCTAssertFalse(NotificationPermission(.denied).asksBeforeOpening)
        XCTAssertFalse(NotificationPermission(.authorized).asksBeforeOpening)
    }

    /// 系统在问过之后只给确定的状态：临时授权和 App Clip 的 ephemeral 一样算允许，
    /// 不然这里当成没问过，每次点都再弹一次框。
    func testAskStateSeparatesNeverAskedFromASystemAnswer() {
        XCTAssertEqual(NotificationPermission(.notDetermined), .notAsked)
        XCTAssertEqual(NotificationPermission(.denied), .denied)
        XCTAssertEqual(NotificationPermission(.authorized), .allowed)
        XCTAssertEqual(NotificationPermission(.provisional), .allowed)
        XCTAssertEqual(NotificationPermission(.ephemeral), .allowed)
    }

    /// 允许时写“已允许”，没问过和拒绝都写“未允许”。
    func testRowValueFollowsThePermission() {
        XCTAssertEqual(NotificationPermission.allowed.rowValue, String(localized: .Settings.notificationAllowed))
        XCTAssertEqual(NotificationPermission.denied.rowValue, String(localized: .Settings.notificationDenied))
        XCTAssertEqual(NotificationPermission.notAsked.rowValue, String(localized: .Settings.notificationDenied))
    }

    /// 落点顺序：文档 API → 带自己 bundle id 的 `App-prefs` 通知页 → 文档里的普通应用页。
    func testSettingsURLsGoFromDocumentedToThePlainAppPage() {
        let urls = NotificationPermission.settingsURLs.map(\.absoluteString)
        XCTAssertEqual(urls.first, UIApplication.openNotificationSettingsURLString)
        XCTAssertEqual(urls.last, UIApplication.openSettingsURLString)
        let bundleID = Bundle.main.bundleIdentifier ?? "?"
        XCTAssertEqual(urls.dropFirst().first, "App-prefs:root=NOTIFICATIONS_ID&path=\(bundleID)")
    }
}
