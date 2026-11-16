import XCTest

/// 开发栈装 whoami 并声明 TCP 映射；PORT_MAPPING_TEST_PORT 给出端口，用真实请求验证监听生命周期。
@MainActor
final class PortMappingUITests: XCTestCase {
    private var app: XCUIApplication!
    private var port: UInt16 = 0

    func testPageAndForegroundControlMapping() async throws {
        continueAfterFailure = false
        executionTimeAllowance = 300
        try UITestBackend.require()
        let configured = ProcessInfo.processInfo.environment["PORT_MAPPING_TEST_PORT"]
        try XCTSkipIf(configured == nil, "Requires the whoami TCP mapping fixture and PORT_MAPPING_TEST_PORT")
        port = try XCTUnwrap(UInt16(configured ?? ""))
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 90), app.debugDescription)
        UITestBackend.settleKeyHandoff(app)
        XCTAssertFalse(app.buttons["key-handoff-skip"].exists)
        XCTAssertFalse(app.buttons["chat-backup-reminder-dismiss"].exists)
        try await expectMapping(false)

        try openPluginPage()
        try await expectMapping(true)

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(
            app.wait(for: .runningBackground, timeout: 10) || app.state == .runningBackgroundSuspended
        )
        try await expectMapping(false)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10))
        try openPluginPage()
        try await expectMapping(true)

        app.buttons["More"].tap()
        let details = app.buttons["View Details"]
        XCTAssertTrue(details.waitForExistence(timeout: 10), app.debugDescription)
        details.tap()
        let state = app.descendants(matching: .any)["portMappingState"].firstMatch
        XCTAssertTrue(state.waitForExistence(timeout: 90), app.debugDescription)
        for _ in 0..<8 where !state.isHittable { app.swipeUp() }
        XCTAssertTrue(state.isHittable)
        XCTAssertFalse(app.descendants(matching: .any)["portMappingReason"].exists)
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "ios-port-map-active"
        attachment.lifetime = .keepAlways
        add(attachment)

        app.buttons["Back"].tap()
        let close = app.buttons["Close"].firstMatch
        XCTAssertTrue(close.waitForExistence(timeout: 10), app.debugDescription)
        close.tap()
        try await expectMapping(false)
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), app.debugDescription)
        XCTAssertFalse(app.webViews.firstMatch.isHittable)
    }

    /// 打开插件页。交接盖层迟到时 `SkipKeyHandoff` 的 `openBackup()` 会把页面栈切到 `ChatBackup`，
    /// 插件页连同映射一起消失；所以每次开好都等一层 `settle`，被顶掉就重开，不拿“当时的栈”当起点。
    private func openPluginPage() throws {
        let url = try XCTUnwrap(URL(string: "wonton://tab/plugin/app.whoami"))
        for _ in 0..<2 {
            app.open(url)
            XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 90), app.debugDescription)
            UITestBackend.settleKeyHandoff(app, timeout: 30)
            if app.webViews.firstMatch.isHittable { return }
        }
        XCTFail("插件页反复被交接盖层顶掉：\(app.debugDescription)")
    }

    private func expectMapping(_ available: Bool) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 1
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/"))
        let deadline = Date().addingTimeInterval(10)
        var reached = false
        repeat {
            do {
                let (data, response) = try await session.data(from: url)
                reached = (response as? HTTPURLResponse)?.statusCode == 200
                    && (String(bytes: data, encoding: .utf8) ?? "").contains("GET / HTTP/")
            } catch {
                reached = false
            }
            if reached == available { return }
            try await Task.sleep(for: .milliseconds(200))
        } while Date() < deadline
        XCTFail("Expected local mapping availability \(available), got \(reached)")
    }
}
