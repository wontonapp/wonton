import XCTest

/// 启动本身：全新安装落在配对页；`WONTON_UI_FAKE` 时核心给假数据，直接进主界面。
/// 两条都不需要家里的服务器，夜跑会跑它们。
final class FirstScreenUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
    }

    func testFreshInstallShowsPairingScreen() throws {
        app.launch()
        XCTAssertTrue(
            app.descendants(matching: .any)["pairing-code"].firstMatch.waitForExistence(timeout: 20),
            "全新安装没有落在配对页"
        )
        XCTAssertTrue(app.buttons["pairing-submit"].exists)
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "fresh-install-pairing"
        shot.lifetime = .keepAlways
        add(shot)
    }

    func testFakeDataShowsChatList() throws {
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(app.staticTexts["回声"].waitForExistence(timeout: 20), "假数据没进聊天列表")
        XCTAssertTrue(app.buttons["Apps"].exists, "主界面没有底栏")
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "fake-chat-list"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
