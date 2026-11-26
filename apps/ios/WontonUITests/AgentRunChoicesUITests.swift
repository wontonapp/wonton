import XCTest

/// Agent 状态行的一串选项（ADR-0082）：多出来的换到下一排、每排等分宽度，照 Telegram 的内联键盘，
/// 任何一个都不挤出屏幕，字也不竖着堆。用夹具屏（`wonton-agent-run-preview`），不需要后端。
final class AgentRunChoicesUITests: XCTestCase {
    private let keys = ["once", "session", "always", "deny"]
    /// 和 `AgentRunPreview.narrowWidth` 一致：最窄 iPhone 的屏宽。
    private let narrowWidth: CGFloat = 320

    func testChoicesWrapIntoEqualRows() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "wonton-agent-run-preview"]
        UITestBackend.launch(app)

        let first = app.buttons["agentRun-choice-once"]
        XCTAssertTrue(first.waitForExistence(timeout: 10), "夹具屏没有选项行\n\(app.debugDescription)")
        let buttons = keys.map { app.buttons["agentRun-choice-\($0)"] }
        for button in buttons {
            XCTAssertTrue(button.exists, "缺一个选项")
        }

        let window = app.windows.element(boundBy: 0).frame
        for button in buttons {
            XCTAssertTrue(
                button.frame.minX >= window.minX && button.frame.maxX <= window.maxX,
                "选项 \(button.identifier) 横向挤出屏幕：\(button.frame) 超出 \(window)"
            )
        }

        // 每排等分：同一排的高度一致、宽度一致；四个选项按个数排成两排。
        let rows = Dictionary(grouping: buttons, by: { $0.frame.minY })
        XCTAssertGreaterThan(rows.count, 1, "四个长选项都在一排：\(buttons.map { $0.frame })")
        for (y, row) in rows {
            let heights = Set(row.map { $0.frame.height })
            XCTAssertEqual(heights.count, 1, "同一排的高度不一致（\(y)）：\(heights)")
            let widths = Set(row.map { $0.frame.width.rounded() })
            XCTAssertEqual(widths.count, 1, "同一排的宽度不相等（\(y)）：\(widths)")
        }

        // 点一下照常发出作答（大字号下可能在屏下方，先滚过来）。
        let session = app.buttons["agentRun-choice-session"]
        try scrollTo(session, in: app)
        session.tap()
        XCTAssertTrue(app.staticTexts["answered session"].waitForExistence(timeout: 5), "点了选项没有作答")

        add(shot(named: "agent-run-choices"))
    }

    /// 最窄的屏配最大辅助字号：仍然两个一排、同排等宽，横着放不挤出去、不竖排。
    /// 设备条件自己带着走：启动参数把字号调到最大、把夹具收窄成最窄 iPhone 的屏宽，不靠人在外面准备。
    func testChoicesOnNarrowScreenAtLargestType() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += [
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
            "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL",
            "wonton-agent-run-preview", "wonton-agent-run-preview-narrow"
        ]
        UITestBackend.launch(app)

        let first = app.buttons["agentRun-choice-once"]
        XCTAssertTrue(first.waitForExistence(timeout: 10), "夹具屏没有选项行\n\(app.debugDescription)")
        let buttons = keys.map { app.buttons["agentRun-choice-\($0)"] }
        for button in buttons {
            XCTAssertTrue(button.exists, "缺一个选项")
        }

        let window = app.windows.element(boundBy: 0).frame
        for button in buttons {
            XCTAssertTrue(
                button.frame.minX >= window.minX && button.frame.maxX <= window.minX + narrowWidth + 1,
                "选项 \(button.identifier) 横向挤出屏幕：\(button.frame) 超出 \(window)"
            )
            XCTAssertLessThan(button.frame.height, button.frame.width, "\(button.identifier) 被压成竖排：\(button.frame)")
        }
        let rows = Dictionary(grouping: buttons, by: { $0.frame.minY })
        XCTAssertEqual(rows.count, 2, "四个选项在窄屏上不是两排：\(buttons.map { $0.frame })")
        for (y, row) in rows {
            XCTAssertEqual(row.count, 2, "一排不是两个（\(y)）")
            let widths = Set(row.map { $0.frame.width.rounded() })
            XCTAssertEqual(widths.count, 1, "同一排的宽度不相等（\(y)）：\(widths)")
        }

        // 最窄屏上也点得到最后一个；滚过去之后顺手把两排都留在图里。
        let deny = app.buttons["agentRun-choice-deny"]
        try scrollTo(deny, in: app)
        add(shot(named: "agent-run-choices-se"))
        deny.tap()
        XCTAssertTrue(app.staticTexts["answered deny"].waitForExistence(timeout: 5), "窄屏上点不到选项")
    }

    /// 窄屏或大字号下选项可能在屏外，点之前先滚过来。
    private func scrollTo(_ element: XCUIElement, in app: XCUIApplication) throws {
        for _ in 0..<4 where !element.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(element.isHittable, "\(element.identifier) 滚不出来")
    }

    private func shot(named name: String) -> XCTAttachment {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        return attachment
    }
}
