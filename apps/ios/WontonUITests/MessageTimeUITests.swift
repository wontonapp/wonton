import XCTest

/// 长按消息的菜单里查得到这一条的发送时间（盲测 #699）：时间按组展示、不贴在气泡上，菜单是单条时间的入口。
/// 不要后端：用 `WONTON_UI_FAKE` 的假数据。
final class MessageTimeUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testContextMenuShowsMessageSendTime() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()

        app.staticTexts["Echo"].tap()
        // 落点在第 2 条附近，第 4 条是整屏的高消息：message 5 在折叠线下面，message 3 一定在屏上。
        let message = app.staticTexts
            .matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 3")).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 10), "时间线没出来")
        message.press(forDuration: 1.2)

        let sentAt = app.descendants(matching: .any)["messageSentAt"]
        XCTAssertTrue(sentAt.waitForExistence(timeout: 5), "长按菜单里没有发送时间\n\(app.debugDescription)")
        XCTAssertTrue(sentAt.label.contains("2026"), "发送时间没带完整日期：\(sentAt.label)")
    }
}
