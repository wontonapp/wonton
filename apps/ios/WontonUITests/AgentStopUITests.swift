import XCTest

/// 输入栏的停止钮（ADR-0061 第 10 节），不要后端（`WONTON_UI_FAKE` 的假数据：发一句话就算 Bot 开了一轮，
/// 停的结果按那句话定，见 `ui_fake_agent_stop.rs`）：Bot 在跑时才有停止钮；点了先是在停、点不了，
/// 停成了说一声、按钮收走；被拒和没送到各有一行说明，按钮留着能再点。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/AgentStopUITests
final class AgentStopUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    private static let room = "chatRoom.!ui-fake:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testStopAppearsWithATurnAndConfirmsTheStop() {
        launch()
        enterRoom()
        XCTAssertFalse(stopButton.exists, "Bot 没在跑时出现了停止钮\n\(app.debugDescription)")

        send("hello")
        XCTAssertTrue(stopButton.waitForExistence(timeout: 5), "Bot 开了一轮，没有停止钮\n\(app.debugDescription)")
        shot("running")

        stopButton.tap()
        XCTAssertTrue(waitUntil { !self.stopButton.isEnabled }, "点了停止，按钮还能再点\n\(app.debugDescription)")
        shot("stopping")
        XCTAssertTrue(note("composer-stop-stopped").waitForExistence(timeout: 5), "停成了没有说明\n\(app.debugDescription)")
        XCTAssertFalse(stopButton.exists, "停成了停止钮还在")
        shot("stopped")
        XCTAssertTrue(waitUntil(timeout: 8) { !self.note("composer-stop-stopped").exists }, "“已停止”一直没收走")
    }

    func testRefusedStopSaysSoAndKeepsTheButton() {
        launch()
        enterRoom()
        send("refuse")
        XCTAssertTrue(stopButton.waitForExistence(timeout: 5), "Bot 开了一轮，没有停止钮\n\(app.debugDescription)")
        stopButton.tap()
        XCTAssertTrue(note("composer-stop-failed").waitForExistence(timeout: 5), "被拒了没有说明\n\(app.debugDescription)")
        XCTAssertTrue(stopButton.isEnabled, "被拒以后停止钮点不了")
        shot("failed")
    }

    func testUnreachableStopIsShownAsUncertain() {
        launch()
        enterRoom()
        send("offline")
        XCTAssertTrue(stopButton.waitForExistence(timeout: 5), "Bot 开了一轮，没有停止钮\n\(app.debugDescription)")
        stopButton.tap()
        XCTAssertTrue(
            note("composer-stop-uncertain").waitForExistence(timeout: 5), "没送到时没有说明\n\(app.debugDescription)"
        )
        XCTAssertFalse(note("composer-stop-stopped").exists, "没送到却说成停了")
        XCTAssertTrue(stopButton.isEnabled, "不确定时停止钮点不了")
        shot("uncertain")
    }

    // MARK: 助手

    private var stopButton: XCUIElement { app.buttons["composer-stop"] }

    private func note(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    private func launch() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
        XCTAssertTrue(
            app.buttons.matching(UITestSidebar.button).firstMatch.waitForExistence(timeout: 20),
            "没进主界面\n\(app.debugDescription)"
        )
    }

    private func enterRoom() {
        let row = app.buttons.matching(NSPredicate(format: "identifier == %@", Self.room)).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 8), "列表里没有 Bot 的房间\n\(app.debugDescription)")
        row.tap()
        XCTAssertTrue(app.textViews["Message"].waitForExistence(timeout: 8), "没进房间\n\(app.debugDescription)")
    }

    private func send(_ text: String) {
        let field = app.textViews["Message"]
        field.tap()
        field.typeText(text)
        let send = app.buttons["Send"]
        XCTAssertTrue(send.waitForExistence(timeout: 5), "没有发送钮\n\(app.debugDescription)")
        send.tap()
    }

    private func waitUntil(timeout: TimeInterval = 5, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return condition()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
