// 消息里的 `wonton:` 链接就地打开，不再交给 Safari 开成白页（ADR-0041、ADR-0065）。
import XCTest

final class ChatAppLinkUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    /// 认不得的 wonton 链接提示一句，不开白页，也不绕系统。
    func testUnknownWontonLinkExplains() {
        enterRoom()
        let link = textView("wonton:tab/from-the-future")
        reveal(link)
        shot("unknown-link")
        link.tap()
        XCTAssertTrue(app.staticTexts["Update the app to open this link"].waitForExistence(timeout: 5),
                      "没有提示\n\(app.debugDescription)")
        shot("unknown-dialog")
        XCTAssertFalse(app.webViews.firstMatch.exists, "打开了 Safari 视图")
    }

    // MARK: 助手

    private func enterRoom() {
        tap(app.buttons["chatRoom.!ui-fake:home.example"])
        XCTAssertTrue(app.staticTexts["Fixture message 3"].waitForExistence(timeout: 10), "时间线没出来")
    }

    /// 链接文字的 UITextView：正文只有这一行链接，点中心就落在链接上。
    private func textView(_ label: String) -> XCUIElement {
        app.textViews.matching(NSPredicate(format: "label == %@", label)).firstMatch
    }

    /// 整个落在顶栏和输入框之间。
    private func inView(_ element: XCUIElement) -> Bool {
        guard element.exists, element.isHittable else { return false }
        let top = app.staticTexts["barActivity"].frame.maxY
        let bottom = app.textViews["Message"].frame.minY
        return element.frame.minY > top && element.frame.maxY < bottom
    }

    /// 从底部一段一段往回翻，直到这个元素整个在可见区里。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<60 {
            if inView(element) { return }
            dragDown()
        }
        XCTFail("翻不到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    private func dragDown() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    private func tap(_ element: XCUIElement) {
        guard wait(20, until: { element.exists && element.isHittable }) else {
            return XCTFail("点不到 \(element)\n\(app.debugDescription)")
        }
        element.tap()
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// 点消息里的 wonton 链接：设置页就地打开，没有 Safari 视图。
    func testWontonLinkOpensInPlace() {
        enterRoom()
        let link = textView("wonton://settings/peers/?from=chat")
        reveal(link)
        shot("app-link")
        link.tap()
        XCTAssertTrue(app.staticTexts["Other Servers"].waitForExistence(timeout: 10), "没进设置页\n\(app.debugDescription)")
        XCTAssertTrue(app.buttons["Add"].waitForExistence(timeout: 10), "「其他服务器」页没有添加按钮\n\(app.debugDescription)")
        XCTAssertFalse(app.webViews.firstMatch.exists, "打开了 Safari 视图")
        shot("peers")
    }
}
