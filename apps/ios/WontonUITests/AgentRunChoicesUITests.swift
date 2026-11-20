import XCTest

/// Agent 状态行的一串选项（ADR-0082）：一排放不下时换成多排、每排等分宽度，照 Telegram 的内联键盘，
/// 任何一个都不挤出屏幕，字也不竖着堆。用夹具屏（`wonton-agent-run-preview`），不需要后端。
final class AgentRunChoicesUITests: XCTestCase {
    private let keys = ["once", "session", "always", "deny"]

    func testChoicesWrapIntoEqualRows() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "wonton-agent-run-preview"]
        app.launch()

        let first = app.buttons["agentRun-choice-once"]
        XCTAssertTrue(first.waitForExistence(timeout: 10), "夹具屏没有选项行\n\(app.debugDescription)")
        let buttons = keys.map { app.buttons["agentRun-choice-\($0)"] }
        for button in buttons {
            XCTAssertTrue(button.exists, "缺一个选项")
        }

        let window = app.windows.element(boundBy: 0).frame
        for button in buttons {
            XCTAssertTrue(
                window.contains(button.frame),
                "选项 \(button.identifier) 挤出屏幕：\(button.frame) 不在 \(window) 里"
            )
        }

        // 每排等分：同一排的高度一致、宽度一致；放不下就得有第二排。
        let rows = Dictionary(grouping: buttons, by: { $0.frame.minY })
        XCTAssertGreaterThan(rows.count, 1, "四个长选项都在一排：\(buttons.map { $0.frame })")
        for (y, row) in rows {
            let heights = Set(row.map { $0.frame.height })
            XCTAssertEqual(heights.count, 1, "同一排的高度不一致（\(y)）：\(heights)")
            let widths = Set(row.map { $0.frame.width.rounded() })
            XCTAssertEqual(widths.count, 1, "同一排的宽度不相等（\(y)）：\(widths)")
        }

        // 点一下照常发出作答。
        let session = app.buttons["agentRun-choice-session"]
        session.tap()
        XCTAssertTrue(app.staticTexts["answered session"].waitForExistence(timeout: 5), "点了选项没有作答")

        add(shot(named: "agent-run-choices"))
    }

    private func shot(named name: String) -> XCTAttachment {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        return attachment
    }
}
