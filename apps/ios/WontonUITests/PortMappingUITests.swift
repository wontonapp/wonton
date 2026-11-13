import XCTest

/// 开发栈装 whoami 并声明 TCP 映射；PORT_MAPPING_TEST_PORT 给出端口，用真实请求验证监听生命周期。
@MainActor
final class PortMappingUITests: XCTestCase {
    private var app: XCUIApplication!
    private var port: UInt16 = 0

    func testPageAndForegroundControlMapping() async throws {
        continueAfterFailure = false
        executionTimeAllowance = 180
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
        try await expectMapping(false)

        app.open(try XCTUnwrap(URL(string: "wonton://tab/plugin/app.whoami")))
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 90), app.debugDescription)
        try await expectMapping(true)

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(
            app.wait(for: .runningBackground, timeout: 10) || app.state == .runningBackgroundSuspended
        )
        try await expectMapping(false)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10))
        UITestBackend.settleKeyHandoff(app)
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
                    && String(decoding: data, as: UTF8.self).contains("GET / HTTP/")
            } catch {
                reached = false
            }
            if reached == available { return }
            try await Task.sleep(for: .milliseconds(200))
        } while Date() < deadline
        XCTFail("Expected local mapping availability \(available), got \(reached)")
    }
}
