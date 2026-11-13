import XCTest

/// Agent 运行状态行（ADR-0082）的夜跑走屏：working 转圈加步骤、一轮收成完成、waiting 整行点开卡片。
/// 不需要后端：启动参数进夹具屏（`AgentRunPreview`，同 QualityPreviews 的做法），时间线要登录后的房间才有。
final class AgentRunUITests: XCTestCase {
    func testWorkingToDoneAndWaitingOpensCard() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "wonton-agent-run-preview"]
        app.launch()

        let working = app.descendants(matching: .any)["agentRun-working"]
        XCTAssertTrue(working.waitForExistence(timeout: 10), "夹具屏没有工作中的状态行\n\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Reading files"].exists, "working 没有显示步骤文字")

        app.buttons["fixture-advance"].tap()
        let done = app.descendants(matching: .any)["agentRun-done"]
        XCTAssertTrue(done.waitForExistence(timeout: 5), "同一轮没有收成完成")
        XCTAssertTrue(app.staticTexts["5 steps"].waitForExistence(timeout: 5), "完成没有步数")

        let waiting = app.descendants(matching: .any)["agentRun-waiting"]
        XCTAssertTrue(waiting.waitForExistence(timeout: 5), "夹具屏没有等确认的状态行")
        waiting.tap()
        XCTAssertTrue(
            app.descendants(matching: .any)["fixture-card"].waitForExistence(timeout: 5),
            "点等确认的行没有打开卡片"
        )

        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "agent-run-preview"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
