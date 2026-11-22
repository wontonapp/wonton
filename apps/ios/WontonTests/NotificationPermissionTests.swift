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
        XCTAssertEqual(NotificationPermission.allowed.rowValue, "Allowed")
        XCTAssertEqual(NotificationPermission.denied.rowValue, "Not allowed")
        XCTAssertEqual(NotificationPermission.notAsked.rowValue, "Not allowed")
    }

    /// 设置页地址带自己的 bundle id，系统才知道开哪一页。
    func testSystemSettingsURLCarriesTheBundleID() {
        let target = NotificationPermission.systemSettingsURL?.absoluteString ?? ""
        XCTAssertTrue(target.hasPrefix("App-Prefs:root="), target)
        XCTAssertTrue(target.hasSuffix(Bundle.main.bundleIdentifier ?? "?"), target)
    }
}
