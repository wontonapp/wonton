import XCTest

/// 房间里到底后往上推出面板（ADR-0052）：不要后端，用 `WONTON_UI_FAKE` 的假数据。
///
/// 假数据的时间线里有几条比屏幕还高的消息，最后一条也是；进房间落在靠前的一条上，点“回到底部”再往上推，
/// 和开发栈上房间走屏的顺序一样。
final class RoomPanelGestureUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// 回到底部后，最后一条消息的最后一行真的在输入框上面；从这里往上推出面板。
    func testPullFromBottomAfterTallMessages() {
        launch()
        let field = enterRoom()
        note("landed \(scrollState)")
        XCTAssertTrue(scrollState.contains("pin=false"), "落点在靠前的一条上，不该贴底：\(scrollState)")
        shot("01-landed")
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch)
        Thread.sleep(forTimeInterval: 2)
        note("after-back-to-bottom \(scrollState)")
        XCTAssertTrue(scrollState.contains("pin=true"), "点了回到底部没贴底：\(scrollState)")
        shot("02-bottom")

        let last = text(startingWith: "Fixture last line")
        XCTAssertTrue(last.waitForExistence(timeout: 5), "回到底部后看不到最后一条的最后一行：\(scrollState)")
        XCTAssertLessThanOrEqual(last.frame.maxY, field.frame.minY + 1,
                                 "最后一行在输入框下面：\(last.frame) 输入框 \(field.frame) \(scrollState)")
        XCTAssertGreaterThan(last.frame.minY, app.frame.midY, "最后一行不在底部：\(last.frame) \(scrollState)")

        pullUp(above: field)
        note("after-pull \(scrollState)")
        shot("03-pulled")
        XCTAssertTrue(app.otherElements["chatPanel"].waitForExistence(timeout: 5),
                      "到底后往上推没有面板：\(scrollState)")
    }

    /// 房间的面板到了整页：网页里面起手的下拉归网页，面板不动；从网页顶边附近起手才退回第 1 级。
    func testFullPanelOnlyReturnsFromThePageEdge() {
        launch()
        let field = enterRoom()
        let safeTop = app.buttons["Back"].frame.minY
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch)
        Thread.sleep(forTimeInterval: 2)
        pullUp(above: field)
        XCTAssertTrue(app.webViews.staticTexts["Fixture player"].waitForExistence(timeout: 20), "面板网页没加载出来")
        XCTAssertTrue(onScreen(field), "往上推一小段就进了整页")
        // 露出的那条聊天页往外推，进整页。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)),
                   withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(wait(5) { !self.onScreen(field) }, "在露出的聊天条上推没有到整页")
        shot("11-room-full")

        dragDown(fromY: app.frame.height * 0.45)
        XCTAssertFalse(wait(2) { self.onScreen(field) }, "网页里面起手的下拉把整页面板拖回了第 1 级")
        shot("12-still-full")

        dragDown(fromY: safeTop + 12)
        XCTAssertTrue(wait(5) { self.onScreen(field) }, "从网页顶边起手的下拉没回第 1 级")
        shot("13-back-to-peek")
    }
}

extension RoomPanelGestureUITests {
    private func launch() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    /// 进假数据里的房间，返回输入框。
    private func enterRoom() -> XCUIElement {
        tap(app.staticTexts["Echo"])
        let field = app.textViews["Message"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "没进房间\n\(app.debugDescription)")
        XCTAssertTrue(text(startingWith: "Fixture message 3").waitForExistence(timeout: 10), "时间线没出来")
        return field
    }

    /// 手指从输入框上面一点往上推到第 1 级：慢拖，不让惯性把列表甩走；不推太远，一口气推过头会直接进整页。
    private func pullUp(above field: XCUIElement) {
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let start = origin.withOffset(CGVector(dx: app.frame.midX, dy: field.frame.minY - 40))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -160)),
                    withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 消息的文字和时间合在一个元素里，按开头找。
    private func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 靠左起手：整页时右上角到中间是胶囊（公网网页的胶囊带域名，更宽），手指落在它上面到不了网页。
    private func dragDown(fromY start: CGFloat) {
        let from = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 48, dy: start))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 220)),
                   withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 输入框还在屏幕里：整页面板把房间整个推出屏幕。
    private func onScreen(_ field: XCUIElement) -> Bool {
        field.exists && field.frame.maxY > app.frame.minY && field.frame.minY < app.frame.maxY
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    /// App 里 `BottomPin` 的自述。
    private var scrollState: String { app.staticTexts["roomScroll"].label }

    private func tap(_ element: XCUIElement) {
        XCTAssertTrue(element.waitForExistence(timeout: 20), "找不到 \(element)\n\(app.debugDescription)")
        element.tap()
    }

    private func note(_ text: String) {
        let attachment = XCTAttachment(string: text)
        attachment.name = text
        attachment.lifetime = .keepAlways
        add(attachment)
        print("NOTE \(text)")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
