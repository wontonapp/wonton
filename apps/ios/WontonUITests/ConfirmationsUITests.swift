import XCTest

/// Real-stack walkthrough: Confirm Probe room, Room approval / Room rejection cards and one
/// No room approval card, plus requests from Contact Approve and Contact Reject.
/// All data comes from the real disposable backend; only PAIRING_CODE is injected.
final class ConfirmationsUITests: XCTestCase {
    private var app: XCUIApplication!
    private let networkWait: TimeInterval = 90

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    func testListAndRoomCards() throws {
        try requireSeededStack()
        reachChat()
        let entry = app.buttons["confirmations-entry"]
        XCTAssertTrue(entry.waitForExistence(timeout: networkWait))
        entry.tap()
        XCTAssertTrue(cardTitle("Room approval").waitForExistence(timeout: networkWait))
        XCTAssertTrue(cardTitle("No room approval").waitForExistence(timeout: networkWait))
        XCTAssertFalse(app.buttons["pending-confirmations-banner"].exists)
        shot("01-confirmations-list")
        back()

        let room = app.buttons.containing(.staticText, identifier: "Confirm Probe").firstMatch
        XCTAssertTrue(room.waitForExistence(timeout: networkWait))
        room.tap()
        let roomBanner = app.buttons["room-confirmations"]
        XCTAssertTrue(roomBanner.waitForExistence(timeout: networkWait))
        roomBanner.tap()
        let approvalCard = expandedCard("Room approval")
        let approve = approvalCard.buttons["Approve room"].firstMatch
        XCTAssertTrue(approve.waitForExistence(timeout: networkWait))
        approve.tap()
        let rejectionCard = expandedCard("Room rejection")
        let reject = rejectionCard.buttons["Reject room"].firstMatch
        if !reject.isHittable { app.swipeUp() }
        XCTAssertTrue(reject.waitForExistence(timeout: networkWait))
        reject.tap()
        shot("02-room-decisions")
        app.swipeDown()
        back()
        // Only the no-origin card remains: its banner must open the list rather than do nothing.
        let banner = app.buttons["pending-confirmations-banner"]
        XCTAssertTrue(banner.waitForExistence(timeout: networkWait))
        banner.tap()
        XCTAssertTrue(cardTitle("No room approval").waitForExistence(timeout: networkWait))
        XCTAssertTrue(cardTitle("Room approval").exists)
        XCTAssertTrue(cardTitle("Room rejection").exists)
        XCTAssertFalse(app.buttons["pending-confirmations-banner"].exists)
        shot("03-finished-list")
    }

    private func expandedCard(_ title: String) -> XCUIElement {
        app.otherElements.matching(NSPredicate(format: "identifier BEGINSWITH %@", "confirmation-expanded-"))
            .containing(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
    }

    private func cardTitle(_ title: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
    }

    private func requireSeededStack() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["CONFIRMATIONS_SEEDED"] == "1",
            "Requires disposable confirmation probe cards and contact requests")
    }

    func testPermanentListEntry() {
        reachChat()
        app.buttons["confirmations-entry"].tap()
        XCTAssertFalse(app.buttons["pending-confirmations-banner"].exists)
        back()
        XCTAssertTrue(app.buttons["confirmations-entry"].exists)
    }

    func testIncomingContactDecisions() throws {
        try requireSeededStack()
        reachChat()
        app.buttons["confirmations-entry"].tap()
        decideContact("Contact Approve", approve: true)
        decideContact("Contact Reject", approve: false)
        shot("04-contact-decisions")
    }

    private func decideContact(_ name: String, approve: Bool) {
        let row = app.otherElements.matching(NSPredicate(format: "identifier BEGINSWITH %@", "confirmation-contact-"))
            .containing(.staticText, identifier: name).firstMatch
        if !row.isHittable { app.swipeUp() }
        XCTAssertTrue(row.waitForExistence(timeout: networkWait))
        let action = row.buttons[approve ? "Approve" : "Reject"].firstMatch
        XCTAssertTrue(action.waitForExistence(timeout: networkWait))
        action.tap()
        XCTAssertTrue(row.waitForNonExistence(timeout: networkWait))
    }

    private func reachChat() {
        let entry = app.buttons["confirmations-entry"]
        for _ in 0..<8 where !entry.waitForExistence(timeout: 10) {
            if UITestBackend.skipKeyHandoff(app) { continue }
            ["Skip", "Done", "Next"].map { app.buttons[$0].firstMatch }
                .first { $0.exists && $0.isEnabled }?.tap()
        }
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(entry.waitForExistence(timeout: networkWait))
    }

    private func back() {
        let button = app.buttons["Back"].firstMatch
        XCTAssertTrue(button.waitForExistence(timeout: 10))
        button.tap()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
