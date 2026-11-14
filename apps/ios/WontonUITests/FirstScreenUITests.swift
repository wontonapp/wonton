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

    @MainActor
    func testUnpairedSidebarEntriesKeepPairingAndRetryAvailable() throws {
        app.launchEnvironment.removeValue(forKey: "WONTON_UI_FAKE")
        app.launch()
        let code = app.descendants(matching: .any)["pairing-code"].firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 10), app.debugDescription)

        try checkSidebarEntries(suffix: "unpaired") {
            XCTAssertTrue(code.waitForExistence(timeout: 10), self.app.debugDescription)
            XCTAssertTrue(self.app.buttons["pairing-submit"].exists)
        }

        var components = try XCTUnwrap(URLComponents(string: "wonton://pair"))
        components.queryItems = [
            URLQueryItem(name: "v", value: "0"),
            URLQueryItem(name: "id", value: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
            URLQueryItem(name: "addr", value: "127.0.0.1:9"),
            URLQueryItem(name: "domain", value: "home.example"),
            URLQueryItem(name: "core", value: "http://home.example/"),
            URLQueryItem(name: "secret", value: String(repeating: "ab", count: 16))
        ]
        let url = try XCTUnwrap(components.url)
        app.open(url)
        expectation(for: NSPredicate(format: "value == %@", url.absoluteString), evaluatedWith: code)
        waitForExpectations(timeout: 10)
        let submit = app.buttons["pairing-submit"]
        XCTAssertTrue(submit.isEnabled)
        submit.tap()
        let failure = app.descendants(matching: .any)["pairing-progress"].firstMatch
        let retry = app.buttons["Try Again with This Code"]
        XCTAssertTrue(retry.waitForExistence(timeout: 90), app.debugDescription)
        XCTAssertTrue(failure.exists, app.debugDescription)

        try checkSidebarEntries(suffix: "unreachable") {
            XCTAssertTrue(failure.waitForExistence(timeout: 10), self.app.debugDescription)
            XCTAssertTrue(retry.isHittable, self.app.debugDescription)
        }
    }

    @MainActor
    private func checkSidebarEntries(suffix: String, verify: () -> Void) throws {
        let destinations = [
            ("chat", ["Chats", "聊天", "チャット"]),
            ("new-chat", ["New Chat", "新聊天", "新建聊天"])
        ]
        for (name, labels) in destinations {
            let row = try XCTUnwrap(UITestSidebar.row(app, labels: labels, timeout: 10), app.debugDescription)
            row.tap()
            verify()
            let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            shot.name = "\(suffix)-\(name)"
            shot.lifetime = .keepAlways
            add(shot)
        }
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
