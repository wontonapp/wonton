import XCTest

/// 房间的通话入口（ADR-0096），不要后端（`WONTON_UI_FAKE` 的假数据：`ui_fake.rs` 里有人在通话的群聊）：
/// 聊天列表那一行标着通话中；进房间顶栏有入口，点了全屏打开 widget 页，页上有出口和中继提示，关掉回到房间；
/// 没装 widget 的房间没有入口。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/CallEntryUITests
///
/// 假数据没有隧道，widget 网页本身加载不出来；真通话在开发栈上走。
final class CallEntryUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    private static let callRoom = "chatRoom.!ui-fake-call:home.example"
    private static let quietRoom = "chatRoom.!ui-fake:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testListRowMarksTheRoomInCall() {
        launch()
        XCTAssertTrue(waitHittable(button(Self.callRoom)), "列表里没有通话中的房间\n\(app.debugDescription)")
        let inCall = hittable(button(Self.callRoom))?.value as? String ?? ""
        XCTAssertFalse(inCall.isEmpty, "通话中的房间那一行没有标记")
        let quiet = hittable(button(Self.quietRoom))?.value as? String ?? ""
        XCTAssertTrue(quiet.isEmpty, "没人通话的房间那一行有通话标记")
        shot("list")
    }

    func testEntryOpensWidgetPageAndExitReturnsToTheRoom() {
        launch()
        tap(button(Self.callRoom), "通话中的房间")
        XCTAssertTrue(waitHittable(button("roomCallActive")), "房间顶栏没有通话中的入口\n\(app.debugDescription)")
        shot("room")

        tap(button("roomCallActive"), "顶栏的通话入口")
        XCTAssertTrue(waitHittable(button("pageClose")), "widget 页没有点得到的出口\n\(app.debugDescription)")
        XCTAssertTrue(
            app.descendants(matching: .any)["callRelayHint"].waitForExistence(timeout: 5),
            "widget 页上没有中继提示\n\(app.debugDescription)"
        )
        shot("widget-page")

        tap(button("pageClose"), "widget 页的出口")
        XCTAssertTrue(waitHittable(button("roomCallActive")), "关掉 widget 页没回到房间\n\(app.debugDescription)")
    }

    func testRoomWithoutWidgetHasNoEntry() {
        launch()
        tap(button(Self.quietRoom), "没装 widget 的房间")
        XCTAssertTrue(waitGone(button(Self.quietRoom)), "没进房间\n\(app.debugDescription)")
        XCTAssertNil(hittable(button("roomCall")), "没有 widget 的房间出现了通话入口")
        XCTAssertNil(hittable(button("roomCallActive")), "没有 widget 的房间出现了通话入口")
    }

    // MARK: 助手

    private func launch() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(
            waitHittable(app.buttons.matching(UITestSidebar.button), timeout: 20), "没进主界面\n\(app.debugDescription)"
        )
    }

    private func button(_ identifier: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier == %@", identifier))
    }

    /// 同名的元素里点得到的那个：被盖住的页面还在界面树里，按名字取第一个会取到它们。
    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) != nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) != nil
    }

    private func waitGone(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) == nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) == nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
