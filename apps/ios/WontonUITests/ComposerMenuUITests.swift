import XCTest

/// 命令列表和回复条（ADR-0054、ADR-0109 第 4、7 节）：点一条命令带着回复对象发出去、草稿不动；列表跟着时间线滚走；
/// 每条命令旁写参数提示。
/// 不要后端：`WONTON_UI_FAKE` 的假房间。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ComposerMenuUITests
final class ComposerMenuUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"
    private static let draft = "send the log first"
    /// 引用摘要两行的高度下限（`typography.supporting` 是 14 / 20，一行放不下第二行的话就算少了）。
    private static let secondLineMinHeight: CGFloat = 34

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    /// 回复着一条、输入框里有草稿时点一条命令：命令带着回复对象发出去，回复条收起，草稿还在。
    func testCommandTapKeepsDraftAndDropsReply() {
        enter()
        startReply()
        let field = app.textViews["Message"]
        field.tap()
        field.typeText(Self.draft)
        openCommands()
        tap(app.buttons["command.model"])

        XCTAssertTrue(text("/model").waitForExistence(timeout: 5), "点命令没发出去\n\(app.debugDescription)")
        XCTAssertTrue(app.buttons["replyCancel"].waitForNonExistence(timeout: 5), "命令发出去以后回复条还在")
        XCTAssertEqual(field.value as? String, Self.draft, "点命令把草稿清掉了")
        screenshot("command-sent")
    }

    /// 列表开着时往上翻时间线：列表收起，让出位置。
    func testCommandMenuCollapsesOnScroll() {
        enter()
        openCommands()
        let menu = app.otherElements["commandMenu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 5), "菜单没打开\n\(app.debugDescription)")
        app.swipeUp()
        XCTAssertTrue(menu.waitForNonExistence(timeout: 5), "滚了时间线命令列表还开着\n\(app.debugDescription)")
    }

    /// 每条命令的名字后面写它的参数提示。
    func testCommandRowShowsHint() {
        enter()
        openCommands()
        XCTAssertTrue(
            app.staticTexts["model name"].waitForExistence(timeout: 5),
            "命令后面没有参数提示\n\(app.debugDescription)"
        )
        screenshot("menu-hint")
    }

    /// 点回复条里的引用：跳回原消息，“向下”出来；跳走不撤回复。
    func testReplyBarJumpsToOriginal() {
        enter()
        startReply()
        screenshot("reply-bar")
        tap(app.buttons["replyQuoteJump"])
        XCTAssertTrue(app.buttons["timelineDown"].waitForExistence(timeout: 10), "点引用没跳走\n\(app.debugDescription)")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "跳走把回复撤了")
    }

    /// 引用的摘要留两行：首行只是一个表情时，第二行也看得见引用的原文（ADR-0109 第 6 节）。
    func testReplyBarKeepsSecondSummaryLine() {
        enter()
        let quoted = app.buttons["messageQuote.ui-fake-20"]
        reveal(quoted)
        let emoji = original("🚗")
        reveal(emoji)
        startReply(on: emoji)
        let reply = app.buttons["replyQuoteJump"]
        let summary = reply.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "好久了")).firstMatch
        XCTAssertTrue(summary.waitForExistence(timeout: 5), "回复条里没有引用的原文\n\(app.debugDescription)")
        XCTAssertGreaterThanOrEqual(summary.frame.height, Self.secondLineMinHeight, "摘要只剩一行：\(summary.frame)")
    }

    // MARK: 助手

    private func enter() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(text("Fixture message 3").waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
    }

    /// 左滑一条消息进入回复；默认拖第 3 条那段字。
    private func startReply(on message: XCUIElement? = nil) {
        let start = (message ?? text("Fixture message 3")).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(
            forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
            withVelocity: .default, thenHoldForDuration: 0.1
        )
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
    }

    private func openCommands() {
        tap(app.buttons["composerCommands"])
    }

    /// 正文以某段字开头的第一个文本，引用里的同一段字不算。
    private func original(_ prefix: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let quotes = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "messageQuote."))
            .allElementsBoundByIndex
        let matches = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix))
            .allElementsBoundByIndex
        guard let original = matches.first(where: { text in
            !quotes.contains { $0.exists && $0.frame.contains(CGPoint(x: text.frame.midX, y: text.frame.midY)) }
        }) else {
            XCTFail("引用以外找不到以“\(prefix)”开头的正文\n\(app.debugDescription)", file: file, line: line)
            return text(prefix)
        }
        return original
    }

    private func text(_ prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 从底部一段一段往回翻，直到这个元素整个在可见区里。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<60 {
            if inView(element) { return }
            dragDown()
        }
        XCTFail("往回翻了 60 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往回翻一段。慢拖、松手前停一下：不带惯性，不会一下翻过头。
    private func dragDown() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        start.press(
            forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)),
            withVelocity: .default, thenHoldForDuration: 0.1
        )
    }

    /// 整个落在顶栏和输入框之间。
    private func inView(_ element: XCUIElement) -> Bool {
        guard element.exists, element.isHittable else { return false }
        let top = app.staticTexts["barActivity"].frame.maxY
        let bottom = app.textViews["Message"].frame.minY
        return element.frame.minY > top && element.frame.maxY < bottom
    }

    private func tap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        guard wait(20, until: { element.exists && element.isHittable }) else {
            return XCTFail("找不到或点不到 \(element)\n\(app.debugDescription)", file: file, line: line)
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

    private func screenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
