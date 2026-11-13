import XCTest

@MainActor
final class PairingScanUITests: XCTestCase {
    private let app = XCUIApplication()

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_TEST_SCAN_INJECTION"] = "1"
        app.launch()
    }

    func testImmediateRefusalAllowsScanningSameCodeAgain() throws {
        let code = try XCTUnwrap(URL(string: "wonton://pair?v=0"))
        try scan(code)
        XCTAssertTrue(element("pairing-refusal").waitForExistence(timeout: 10))
        XCTAssertFalse(element("pairing-code").exists)
        capture("scan-refused")
        app.buttons["pairing-scan-again"].tap()
        XCTAssertTrue(element("pairing-code").waitForExistence(timeout: 10))
        try scan(code)
        XCTAssertTrue(element("pairing-refusal").waitForExistence(timeout: 10))
        capture("same-code-rescanned")
    }

    func testUnreachableServerShowsProgressThenAllowsRescan() throws {
        let code = try unreachableCode()
        try scan(code)
        XCTAssertTrue(element("pairing-connecting").waitForExistence(timeout: 10))
        XCTAssertFalse(element("pairing-code").exists)
        XCTAssertFalse(app.buttons["pairing-inject-scan"].exists)
        capture("scan-connecting")
        XCTAssertTrue(app.buttons["pairing-scan-again"].waitForExistence(timeout: 90))
        capture("scan-network-failure")
        app.buttons["pairing-scan-again"].tap()
        XCTAssertTrue(element("pairing-code").waitForExistence(timeout: 10))
        try scan(code)
        XCTAssertTrue(element("pairing-connecting").waitForExistence(timeout: 10))
        capture("scan-reconnecting")
    }

    /// Supply PAIRING_CODE after building; this does not use the auto-pair launch shortcut.
    func testSuppliedServerCode() throws {
        try UITestBackend.require()
        let code = try XCTUnwrap(try UITestBackend.codes().first)
        try scan(try XCTUnwrap(URL(string: code)))
        capture("server-code-recognized")
        if ProcessInfo.processInfo.environment["PAIRING_SCAN_EXPECT_FAILURE"] == "1" {
            XCTAssertTrue(app.buttons["pairing-scan-again"].waitForExistence(timeout: 90))
            capture("server-code-failure")
            app.buttons["pairing-scan-again"].tap()
            XCTAssertTrue(element("pairing-code").waitForExistence(timeout: 10))
        } else {
            let paired = NSPredicate { [self] _, _ in
                !element("pairing-progress").exists && !element("pairing-code").exists
                    && !element("pairing-refusal").exists
                    && (app.buttons["key-handoff-skip"].exists
                        || app.buttons.matching(UITestSidebar.button).firstMatch.exists)
            }
            expectation(for: paired, evaluatedWith: app)
            waitForExpectations(timeout: 90)
            XCTAssertFalse(app.buttons["pairing-scan-again"].exists)
            capture("server-code-paired")
        }
    }

    private func scan(_ code: URL) throws {
        XCTAssertTrue(element("pairing-code").waitForExistence(timeout: 10))
        app.open(code)
        expectation(for: NSPredicate(format: "value == %@", code.absoluteString),
                    evaluatedWith: element("pairing-code"))
        waitForExpectations(timeout: 10)
        let inject = app.buttons["pairing-inject-scan"]
        XCTAssertTrue(inject.waitForExistence(timeout: 10))
        inject.tap()
    }

    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier].firstMatch
    }

    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func unreachableCode() throws -> URL {
        var code = try XCTUnwrap(URLComponents(string: "wonton://pair"))
        code.queryItems = [
            URLQueryItem(name: "v", value: "0"),
            URLQueryItem(name: "id", value: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
            URLQueryItem(name: "addr", value: "127.0.0.1:9"),
            URLQueryItem(name: "domain", value: "home.example"),
            URLQueryItem(name: "core", value: "http://home.example/"),
            URLQueryItem(name: "secret", value: String(repeating: "ab", count: 16))
        ]
        return try XCTUnwrap(code.url)
    }
}
