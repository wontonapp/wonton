import XCTest

/// 转发面板：多选会话、附言、附言在目标房间里的先后顺序。
final class ChatTelegramForwardUITests: ChatTelegramUITestSupport {
    /// 转发：面板顶上能搜，会话能选几个；选了以后附言框和发送键贴着面板底、点得到，写了附言点发送面板收起。
    func testForwardSheetSelectsSeveralAndSendsWithComment() {
        enter(Self.directRoom)
        openForwardSheet()
        let send = app.buttons["forwardSend"]
        XCTAssertFalse(send.exists, "还没选就有发送键")
        selectTwoRooms()
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了会话，发送键不在屏幕内\n\(app.debugDescription)")
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture comment")
        XCTAssertTrue(wait(5) { send.isHittable }, "键盘弹起后发送键点不到")
        shot("forward-sheet")
        send.tap()
        XCTAssertTrue(app.textFields["forwardSearch"].waitForNonExistence(timeout: 5), "发送以后面板没收起")
    }

    /// 转发时写了附言：附言和转发的原消息都进了对方房间，附言排在上面（ADR-0073 第 4 节、ADR-0109 第 9 节）。
    func testForwardCommentLandsInTargetRoom() {
        enter(Self.directRoom)
        openForwardSheet()
        let search = app.textFields["forwardSearch"]
        focus(search)
        search.typeText("Notes")
        tap(app.buttons["forward-!ui-fake-notes:home.example"])
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture forward comment")
        XCTAssertEqual(comment.value as? String, "Fixture forward comment", "附言没打进附言框")
        tap(app.buttons["forwardSend"])
        // 附言在目标房间没打开时排着，打开那条才发出去。
        XCTAssertTrue(app.buttons["forwardSend"].waitForNonExistence(timeout: 5), "转发面板没收起")
        tap(app.buttons["Back"])
        XCTAssertTrue(app.buttons["chatRoom.!ui-fake-notes:home.example"].waitForExistence(timeout: 10),
                      "没回到会话列表\n\(app.debugDescription)")
        enter("chatRoom.!ui-fake-notes:home.example")
        jumpToNewest()
        // 副本是最后一条、比整屏还高，贴底时它就在屏上；附言在它上面一条，往回翻一点就进树。
        // 原件滚到顶上去了，所以不能按“两条 Fixture message 4”数，按先后比。
        let copy = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 4")).firstMatch
        let arrived = text(startingWith: "Fixture forward comment")
        revealBackwards(arrived)
        XCTAssertTrue(arrived.exists, "附言没进目标房间\n\(app.debugDescription)")
        XCTAssertTrue(copy.exists, "附言下面没有转发的消息\n\(app.debugDescription)")
        XCTAssertLessThan(arrived.frame.minY, copy.frame.minY, "转发的消息没排在附言下面")
        shot("forwarded-comment")
        darkShot("forwarded-comment-dark")
    }

    /// 转发带附言发给两个会话：两边各有一条附言压在自己的副本上面（ADR-0073 第 4 节、ADR-0109 第 9 节）。
    /// 顺序按时间线里两条消息的先后比，只看都在不够。
    func testForwardCommentToEveryRoomLandsAboveItsCopy() {
        enter(Self.directRoom)
        openForwardSheet()
        let search = app.textFields["forwardSearch"]
        focus(search)
        search.typeText("am")
        tap(app.buttons["forward-!ui-fake-call:home.example"])
        // 清掉搜索再找另一个房间。
        focus(search)
        search.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2))
        search.typeText("Notes")
        tap(app.buttons["forward-!ui-fake-notes:home.example"])
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture forward comment")
        tap(app.buttons["forwardSend"])
        XCTAssertTrue(app.textFields["forwardSearch"].waitForNonExistence(timeout: 5), "转发面板没收起")
        for (id, name) in [("!ui-fake-call:home.example", "Family"), ("!ui-fake-notes:home.example", "Notes")] {
            tap(app.buttons["Back"])
            XCTAssertTrue(app.buttons["chatRoom.\(id)"].waitForExistence(timeout: 10),
                          "没回到会话列表\n\(app.debugDescription)")
            enter("chatRoom.\(id)")
            jumpToNewest()
            // 副本是最后一条、比整屏还高，贴底时它就在屏上；附言在它上面一条，往回翻一点就进树。
            let copy = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 4")).firstMatch
            let arrived = text(startingWith: "Fixture forward comment")
            revealBackwards(arrived)
            XCTAssertTrue(arrived.exists, "\(name) 里没有附言\n\(app.debugDescription)")
            XCTAssertTrue(copy.exists, "\(name) 里没有副本\n\(app.debugDescription)")
            XCTAssertLessThan(arrived.frame.minY, copy.frame.minY, "\(name) 里附言没排在副本上面")
            shot("forward-comment-\(name.lowercased())")
        }
    }

    // MARK: 助手

    /// 回到底部：有“向下”就一直点，点没了就是贴底了。
    private func jumpToNewest() {
        for _ in 0..<20 where app.buttons["timelineDown"].exists { app.buttons["timelineDown"].tap() }
    }

    /// 从底部往回翻到 `element` 进了树为止。长消息比整屏还高、命不中，只看存在性。
    private func revealBackwards(_ element: XCUIElement) {
        for _ in 0..<40 where !element.exists { dragDown() }
    }
}
