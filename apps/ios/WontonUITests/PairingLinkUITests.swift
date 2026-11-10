import XCTest
import WontonClient

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

        XCTAssertTrue(code.waitForExistence(timeout: 5))
        XCTAssertEqual(code.value as? String, url.absoluteString)
        let submit = app.buttons["pairing-submit"]
        XCTAssertTrue(submit.isEnabled)
        XCTAssertFalse(app.descendants(matching: .any)["pairing-refusal"].firstMatch.exists)
        XCTAssertFalse(app.descendants(matching: .any)["pairing-progress"].firstMatch.exists)
        submit.tap()

        let refusal = app.descendants(matching: .any)["pairing-refusal"].firstMatch
        XCTAssertTrue(refusal.waitForExistence(timeout: 5))
        let reason = try rejectionReason(for: url.absoluteString)
        XCTAssertTrue(refusal.staticTexts[reason].waitForExistence(timeout: 5))
        XCTAssertFalse(app.descendants(matching: .any)["pairing-progress"].firstMatch.exists)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "malformed-pairing-link-rejected-after-confirmation"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private func rejectionReason(for code: String) throws -> String {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let core = try ClientCore(dataDir: directory.path)
        var reason: String?
        do {
            try core.dispatch(action: .pairHome(code: code, deviceName: "Test phone"))
        } catch let CoreError.InvalidInput(value) {
            reason = value
        }
        return try XCTUnwrap(reason)
    }
}
