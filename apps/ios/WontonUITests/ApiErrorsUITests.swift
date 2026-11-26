import XCTest

/// 核心报错页（notReady、invalidInput）把原文收进“详情”（AGENTS.md“界面设计”：正文一句人话，原文默认收起）：
/// `notReady`、`invalidInput` 的正文只有壳自己的下一步，原文藏在折叠的详情里。
/// 用夹具屏（`wonton-problem-preview`），不需要后端。
final class ApiErrorsUITests: XCTestCase {
    private let details = [
        "problemFixture-notReady": "The sync session is not ready yet (M_UNKNOWN, since 14:02)",
        "problemFixture-invalidInput": "Can't build a reply to this message: M_NOT_FOUND"
    ]

    func testCoreTextStartsCollapsedUnderDetails() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "wonton-problem-preview"]
        UITestBackend.launch(app)

        let notReady = app.descendants(matching: .any)["problemFixture-notReady"]
        XCTAssertTrue(notReady.waitForExistence(timeout: 10), "夹具屏没有出错页\n\(app.debugDescription)")

        // 标题是壳自己的类别话，原文一条都不在正文里。
        XCTAssertTrue(app.staticTexts["Not available right now"].exists, "notReady 没有自己的标题")
        XCTAssertTrue(app.staticTexts["This can't be done"].exists, "invalidInput 没有自己的标题")
        for text in details.values {
            XCTAssertFalse(app.staticTexts[text].exists, "核心原文还在正文里：\(text)")
        }

        // 一张“详情”给一个类别，点开才露出原文。
        let toggles = app.buttons.matching(NSPredicate(format: "label == 'Details'"))
        XCTAssertEqual(toggles.count, 2, "每张出错页都该有一个默认收起的详情\n\(app.debugDescription)")
        try tapVisible(toggles.element(boundBy: 0), in: app)
        XCTAssertTrue(
            app.staticTexts[details["problemFixture-notReady"]!].waitForExistence(timeout: 5),
            "点开详情没有露出核心原文"
        )
        // 另一张不受影响，仍是收起的。
        XCTAssertFalse(app.staticTexts[details["problemFixture-invalidInput"]!].exists, "两张详情连到了一起")
        try tapVisible(toggles.element(boundBy: 1), in: app)
        XCTAssertTrue(
            app.staticTexts[details["problemFixture-invalidInput"]!].waitForExistence(timeout: 5),
            "第二张详情点不开"
        )

        add(shot(named: "problem-details-open"))
    }

    /// 收起时的样子单独留一张：给 PR 看默认状态。
    func testCollapsedShot() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "wonton-problem-preview"]
        UITestBackend.launch(app)
        XCTAssertTrue(
            app.descendants(matching: .any)["problemFixture-notReady"].waitForExistence(timeout: 10),
            "夹具屏没有出错页"
        )
        add(shot(named: "problem-details-collapsed"))
    }

    /// 第二张出错页在下面，点之前先滚到能点，免得点到屏幕外。
    private func tapVisible(_ element: XCUIElement, in app: XCUIApplication) throws {
        XCTAssertTrue(element.waitForExistence(timeout: 5), "找不到详情按钮")
        for _ in 0..<3 where !element.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(element.isHittable, "详情按钮滚不出来")
        element.tap()
    }

    private func shot(named name: String) -> XCTAttachment {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        return attachment
    }
}
