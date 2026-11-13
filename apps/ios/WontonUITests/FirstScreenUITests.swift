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
        continueAfterFailure = false
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
        continueAfterFailure = false
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(
            app.staticTexts["Echo"].waitForExistence(timeout: 20),
            "假数据没进聊天列表\n\(app.debugDescription)"
        )
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "主界面没有侧边栏入口\n\(app.debugDescription)")
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "fake-chat-list"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
