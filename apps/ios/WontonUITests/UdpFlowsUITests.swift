import XCTest

@MainActor
final class UdpFlowsUITests: XCTestCase {
    func testFlowDetailsExpandAndCollapse() {
        continueAfterFailure = false
        let app = launch("udp")
        let summary = app.buttons["udpFlowsSummary"]
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        let first = app.descendants(matching: .any)["udpFlow-1"].firstMatch
        let second = app.descendants(matching: .any)["udpFlow-2"].firstMatch
        XCTAssertFalse(first.exists)
        summary.tap()
        XCTAssertTrue(first.waitForExistence(timeout: 10))
        XCTAssertTrue(second.exists)
        XCTAssertTrue(first.label.contains("7882"), first.label)
        XCTAssertTrue(second.label.contains("52000"), second.label)
        attach("udp-flows-expanded")
        summary.tap()
        XCTAssertFalse(first.exists)
        XCTAssertFalse(second.exists)
    }

    func testEmptyFlowsHideSummaryAndUnknownDropsStayAbsent() {
        continueAfterFailure = false
        let empty = launch("details")
        XCTAssertFalse(empty.buttons["udpFlowsSummary"].exists)
        empty.terminate()

        let unknown = launch("udp-unknown")
        let unknownSummary = unknown.buttons["udpFlowsSummary"]
        XCTAssertTrue(unknownSummary.waitForExistence(timeout: 10))
        let unknownLabel = unknownSummary.label
        unknown.terminate()

        let zero = launch("udp-zero")
        let zeroSummary = zero.buttons["udpFlowsSummary"]
        XCTAssertTrue(zeroSummary.waitForExistence(timeout: 10))
        XCTAssertEqual(zeroSummary.label, unknownLabel)
    }

    func testUnsignedCountersKeepTheirFullRange() {
        continueAfterFailure = false
        let app = launch("udp-large")
        let summary = app.buttons["udpFlowsSummary"]
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        XCTAssertTrue(summary.label.filter(\.isNumber).contains(String(UInt64.max)), summary.label)
        summary.tap()
        let second = app.descendants(matching: .any)["udpFlow-2"].firstMatch
        XCTAssertTrue(second.waitForExistence(timeout: 10))
        let digits = second.label.filter(\.isNumber)
        XCTAssertTrue(digits.contains(String(UInt64.max)), second.label)
        attach("udp-flows-unsigned-counters")
    }

    private func launch(_ scenario: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = [
            "wonton-quality-preview-\(scenario)", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"
        ]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launch()
        return app
    }

    private func attach(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
