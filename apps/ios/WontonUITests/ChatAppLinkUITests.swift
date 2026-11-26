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
        UITestBackend.launch(app)
    }

    /// 认不得的 wonton 链接提示一句，不开白页，也不绕系统。
    func testUnknownWontonLinkExplains() {
        enterRoom()
        let link = message("ui-fake-7")
        reveal(link)
        shot("unknown-link")
        tapLink("wonton:tab/from-the-future", link)
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

    /// 带链接的那条消息：正文只有这一行链接。
    private func message(_ id: String) -> XCUIElement {
        app.otherElements["message.\(id)"]
    }

    /// 点消息里的链接：链接有自己的元素就点它，没有就点气泡最下面那行说明（照片消息的链接在说明上）。
    private func tapLink(_ url: String, _ bubble: XCUIElement) {
        let link = app.links.matching(NSPredicate(format: "label == %@", url)).firstMatch
        // 说明行是文本视图的子元素，气泡进屏以后还要等它排版出来。
        if link.waitForExistence(timeout: 3), link.isHittable {
            // 点字的左端：行中心压着文本视图的滚动条，直接 tap 中心会被它吃掉。
            return link.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.35)).tap()
        }
        let body = app.textViews.matching(NSPredicate(format: "label == %@", url)).firstMatch
        if body.exists, body.isHittable { return body.tap() }
        let plain = app.staticTexts.matching(NSPredicate(format: "label == %@", url)).firstMatch
        if plain.exists, plain.isHittable { return plain.tap() }
        print("tapLink 没有链接元素：links=\(app.links.count) bubble=\(bubble.frame)")
        bubble.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.85)).tap()
    }

    /// 整个落在顶栏和输入框之间。
    private func inView(_ element: XCUIElement) -> Bool {
        guard element.exists, element.isHittable else { return false }
        let top = app.staticTexts["barActivity"].frame.maxY
        let bottom = app.textViews["Message"].frame.minY
        return element.frame.minY > top && element.frame.maxY < bottom
    }

    /// 一段一段翻到元素整个在可见区里：落点在第 2 条，链接在它新的一头，没画出来时先往那边翻；
    /// 画到顶栏上面就换回来，翻到头（列表不动了）也换向。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        let top = app.staticTexts["barActivity"].frame.maxY
        var up = true
        var lastY = CGFloat.nan
        var stuck = 0
        for step in 0..<120 {
            if inView(element) { return }
            if step.isMultiple(of: 10) { print("reveal: 第 \(step) 段 up=\(up)") }
            if element.exists, element.frame.minY < top { up = false }
            if up { dragUp() } else { dragDown() }
            let y = app.collectionViews.firstMatch.cells.firstMatch.frame.minY
            if y == lastY {
                stuck += 1
            } else {
                stuck = 0
                lastY = y
            }
            if stuck >= 3 {
                print("reveal: 翻不动了，换向 up=\(!up)")
                up.toggle()
                stuck = 0
            }
        }
        XCTFail("两头都没翻到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往新的一头翻一小段。
    private func dragUp() {
        drag(from: CGVector(dx: 0.5, dy: 0.6), to: CGVector(dx: 0.5, dy: 0.35))
    }

    /// 往旧的一头翻一小段。
    private func dragDown() {
        drag(from: CGVector(dx: 0.5, dy: 0.35), to: CGVector(dx: 0.5, dy: 0.6))
    }

    private func drag(from: CGVector, to: CGVector) {
        app.coordinate(withNormalizedOffset: from)
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: to),
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
        let link = message("ui-fake-6")
        reveal(link)
        shot("app-link")
        tapLink("wonton://settings/peers/?from=chat", link)
        XCTAssertTrue(app.staticTexts["Other Servers"].waitForExistence(timeout: 10), "没进设置页\n\(app.debugDescription)")
        XCTAssertTrue(app.buttons["Add"].waitForExistence(timeout: 10), "「其他服务器」页没有添加按钮\n\(app.debugDescription)")
        XCTAssertFalse(app.webViews.firstMatch.exists, "打开了 Safari 视图")
        shot("peers")
    }
}
