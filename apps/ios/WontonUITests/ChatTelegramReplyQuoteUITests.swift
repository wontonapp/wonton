import XCTest

/// 左滑回复、点引用跳回原消息和“向下”的返回栈、引用与长消息的排版。
final class ChatTelegramReplyQuoteUITests: ChatTelegramUITestSupport {
    /// 向左拖一条消息，松手后输入框上方出现回复条。
    func testSwipeLeftStartsReply() {
        enter(Self.directRoom)
        XCTAssertFalse(app.buttons["replyCancel"].exists, "还没拖就在回复")
        let start = message(3).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
                    withVelocity: .default, thenHoldForDuration: 0.1)
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
        shot("reply-bar")
    }

    /// 点气泡里的引用跳到原消息；“向下”先回到跳之前那一条，再点一次才到底。
    func testQuoteJumpsToOriginalAndDownGoesBackFirst() {
        enter(Self.directRoom)
        let down = app.buttons["timelineDown"]
        let quote = app.buttons["messageQuote.ui-fake-21"]
        tap(down)
        reveal(quote)
        XCTAssertFalse(originalOnScreen(5, besides: quote), "还没跳，原消息就在屏上\n\(app.debugDescription)")
        quote.tap()
        XCTAssertTrue(wait(10) { self.originalOnScreen(5, besides: quote) }, "点引用没跳到原消息\n\(app.debugDescription)")
        shot("jump-highlight")
        XCTAssertFalse(inView(quote), "跳走以后被点的那条还在屏上")

        tap(down)
        XCTAssertTrue(wait(10) { self.inView(quote) }, "“向下”没先回到跳之前那一条\n\(app.debugDescription)")
        tap(down)
        let last = text(startingWith: "Fixture last line")
        XCTAssertTrue(wait(10) { last.exists && last.isHittable }, "返回栈空了以后“向下”没到底")
    }

    /// 跳转本身也是一路在滚：途中要出浮动日期；手一碰打断跳转，也要在原消息上闪一下点名（ADR-0109 第 4 节）。
    func testQuoteJumpShowsDateAndStillFlashesWhenTouched() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-21"]
        reveal(quote)
        let scroll = app.descendants(matching: .any)["roomScroll"]
        let before = flashCount(scroll)
        quote.tap()
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForExistence(timeout: 3), "跳的途中没有浮动日期\n\(app.debugDescription)")
        // 立刻碰一下列表：程序化滚动停下，闪光照样要点在原消息上。
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 20)),
                    withVelocity: .default, thenHoldForDuration: 0.05)
        // 闪光只亮一秒多，读得比它晚：按永远留着的计数和最后一行核对。
        XCTAssertTrue(wait(5) { flashCount(scroll) > before },
                      "手碰断跳转后原消息没闪\n\(app.debugDescription)")
        XCTAssertTrue(scroll.label.contains("last=ui-fake-5"), "闪的不是原消息\n\(app.debugDescription)")
    }

    /// 引用摘要按原文换行，两行都给，还是最多两行（和安卓一样）；输入框上方那条回复也是两行。
    func testQuoteSummaryKeepsBothLines() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-20"]
        reveal(quote)
        // 名字一行加摘要两行约 58；只剩一行约 38。
        XCTAssertGreaterThanOrEqual(quote.frame.height, 56, "引用摘要只剩一行\n\(app.debugDescription)")
        XCTAssertLessThanOrEqual(quote.frame.height, Self.quoteMaxHeight, "引用摘要超过两行")
        waitForFloatingDateToFade()
        shot("quote-two-lines")
    }

    /// 引用跳到那条“表情 + 一行字”的消息，左滑它：输入框上方的回复条也要两行。
    func testComposerReplySummaryKeepsBothLines() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-20"]
        reveal(quote)
        quote.tap()
        let original = app.descendants(matching: .any)["message.ui-fake-19"]
        XCTAssertTrue(wait(10) { self.inView(original) }, "跳转没到原消息\n\(app.debugDescription)")
        let start = original.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
                    withVelocity: .default, thenHoldForDuration: 0.1)
        let reply = app.descendants(matching: .any)["composerReplyQuote"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
        // 名字一行加摘要两行约 54；只剩一行约 38。
        XCTAssertGreaterThanOrEqual(reply.frame.height, 50, "输入框上的摘要只剩一行\n\(app.debugDescription)")
        shot("reply-two-lines")
    }

    /// 引用块只有名字加一两行摘要那么高，不跟着气泡撑开。
    func testQuoteBlockHugsItsContent() {
        enter(Self.directRoom)
        let quote = app.buttons["messageQuote.ui-fake-20"]
        tap(app.buttons["timelineDown"])
        reveal(quote)
        waitForFloatingDateToFade()
        shot("long-reply")
        XCTAssertLessThanOrEqual(quote.frame.height, Self.quoteMaxHeight, "引用块太高：\(quote.frame)")
    }

    /// 长消息每一段都完整显示，不按行数截断；时间只落在最后一段的末行，不压着别的字。
    func testLongReplyShowsEveryLineAndTimeStaysClear() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        reveal(app.buttons["messageQuote.ui-fake-20"])
        let paragraphs = Self.longReplyParagraphs.map { text(startingWith: $0.prefix) }
        for (paragraph, expected) in zip(paragraphs, Self.longReplyParagraphs) {
            XCTAssertTrue(paragraph.waitForExistence(timeout: 5), "找不到“\(expected.prefix)”这一段")
            // 一个汉字宽不超过正文字号：这一段至少要这么多行。被截断的段行数不够。
            let lines = (CGFloat(expected.hanCount) * Self.hanWidth / paragraph.frame.width).rounded(.up)
            XCTAssertGreaterThanOrEqual(
                paragraph.frame.height, lines * Self.lineHeight - 2,
                "“\(expected.prefix)”这一段被截断：\(paragraph.frame)，至少要 \(lines) 行"
            )
        }
        let last = paragraphs[paragraphs.count - 1].frame
        let metas = app.descendants(matching: .any).matching(identifier: "messageMeta").allElementsBoundByIndex
            .map { $0.frame }.filter { $0.intersects(last) }
        XCTAssertEqual(metas.count, 1, "最后一段上应该正好有一处时间：\(metas)")
        for meta in metas {
            XCTAssertGreaterThanOrEqual(meta.minY, last.maxY - Self.lineHeight * 1.5, "时间不在最后一段的末行：\(meta) \(last)")
            for paragraph in paragraphs.dropLast() {
                XCTAssertFalse(meta.intersects(paragraph.frame), "时间压在前面的段上：\(meta) \(paragraph.frame)")
            }
        }
    }

    /// 假房间第 20 条（核心的 `ui_fake_text/long_reply.txt`）每段开头几个字和汉字个数的下限。
    private static let longReplyParagraphs: [(prefix: String, hanCount: Int)] = [
        ("没有出问题", 20), ("拖得久的原因", 80), ("有一件我确实做不到的事", 50), ("你在 App 里打开", 40)
    ]
    /// 默认字号下正文的字宽下限和行高（`typography.message` 是 16 / 21，字宽留一点余量）。
    private static let hanWidth: CGFloat = 15
    private static let lineHeight: CGFloat = 21
    /// 引用块最高：名字一行（18）加摘要两行（各 20），再留一点余量。
    private static let quoteMaxHeight: CGFloat = 64

    // MARK: 助手

    /// 第几条消息自己在屏上、点得到。引用里的摘要和原消息是一样的字，落在引用里的不算。
    private func originalOnScreen(_ index: Int, besides quote: XCUIElement) -> Bool {
        let inQuote = quote.exists ? quote.frame : .null
        return app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message \(index)"))
            .allElementsBoundByIndex
            .contains { inView($0) && !inQuote.contains(CGPoint(x: $0.frame.midX, y: $0.frame.midY)) }
    }

    /// `ScrollDebug` 里的闪光计数；读不到给 -1。
    private func flashCount(_ element: XCUIElement) -> Int {
        let part = element.label.split(separator: " ").first { $0.hasPrefix("flashes=") }
        return part.flatMap { Int($0.dropFirst("flashes=".count)) } ?? -1
    }
}
