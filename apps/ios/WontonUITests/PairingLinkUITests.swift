import XCTest

@MainActor
final class PairingLinkUITests: XCTestCase {
    func testMalformedLinkWaitsForConfirmationAndShowsItsReason() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launch()
        let code = app.descendants(matching: .any)["pairing-code"].firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 15))
        let url = try XCTUnwrap(URL(string: "wonton://pair?v=0"))

        app.open(url)

        expectation(for: NSPredicate(format: "value == %@", url.absoluteString), evaluatedWith: code)
        waitForExpectations(timeout: 15)
        let submit = app.buttons["pairing-submit"]
        XCTAssertTrue(submit.isEnabled)
        XCTAssertFalse(app.descendants(matching: .any)["pairing-refusal"].firstMatch.exists)
        XCTAssertFalse(app.descendants(matching: .any)["pairing-progress"].firstMatch.exists)
        submit.tap()

        let refusal = app.descendants(matching: .any)["pairing-refusal"].firstMatch
        XCTAssertTrue(refusal.waitForExistence(timeout: 5))
        XCTAssertFalse(app.descendants(matching: .any)["pairing-progress"].firstMatch.exists)
        XCTAssertFalse(app.descendants(matching: .any)["pairing-code"].firstMatch.exists)
        app.buttons["pairing-scan-again"].tap()
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertFalse(refusal.exists)
        XCTAssertFalse(submit.isEnabled)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "malformed-pairing-link-rejected-after-confirmation"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

}
