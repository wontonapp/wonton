import XCTest

/// 聊天页照 Telegram 的几样交互（ADR-0109）：左滑回复、点引用跳到原消息、“向下”先回到跳之前那一条、
/// “/”命令菜单、消息下的选项按钮、顶栏的正在输入、群聊里组尾的头像。
/// 不要后端：用 `WONTON_UI_FAKE` 的假房间（核心的 `ui_fake_room.rs` 给条目，`ui_fake_timeline.rs` 响应操作）。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ChatTelegramUITests
final class ChatTelegramUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"
    private static let groupRoom = "chatRoom.!ui-fake-call:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        // 不恢复上一次的场景状态：房间记着上次看到哪一条（SceneStorage），上一条用例翻到别处会带到这一条来。
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

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

    /// 选项按钮：点一个变成选中，另一个不能再点。
    func testChoiceButtonsAnswerOnce() {
        enter(Self.directRoom)
        let approve = app.buttons["messageChoice.✅"]
        let deny = app.buttons["messageChoice.❌"]
        tap(app.buttons["timelineDown"])
        reveal(approve)
        XCTAssertTrue(approve.isEnabled && deny.isEnabled, "还没选，按钮就点不了")
        XCTAssertFalse(approve.isSelected || deny.isSelected, "还没选，就有按钮是选中的")
        waitForFloatingDateToFade()
        shot("choices-open")
        approve.tap()
        XCTAssertTrue(wait(5) { approve.isSelected }, "点了的选项没变成选中")
        XCTAssertFalse(deny.isSelected, "没点的选项成了选中")
        XCTAssertFalse(deny.isEnabled, "选过以后另一个还能点")
        shot("choices-answered")
    }

    /// 输入框开头敲“/”弹出命令列表，按前缀过滤；点一条直接发出去，输入框里的草稿留着、列表收起；“菜单”按钮打开同一张列表。
    func testSlashOpensCommandMenuAndTapSends() {
        enter(Self.directRoom)
        let field = input
        let commands = ["help", "model", "new"].map { app.buttons["command.\($0)"] }
        XCTAssertFalse(commands[0].exists, "没敲“/”就有命令列表")
        focus(field)
        field.typeText("/")
        for command in commands {
            XCTAssertTrue(command.waitForExistence(timeout: 5), "敲“/”后列表里少了 \(command)")
        }
        shot("command-menu")
        field.typeText("m")
        XCTAssertTrue(commands[0].waitForNonExistence(timeout: 5), "按前缀过滤后不匹配的命令还在")
        XCTAssertTrue(commands[1].exists, "按前缀过滤把匹配的命令也滤掉了")
        field.typeText(XCUIKeyboardKey.delete.rawValue)
        tap(commands[0])
        XCTAssertTrue(text(startingWith: "/help").waitForExistence(timeout: 5), "点了命令，时间线末尾没有这条消息")
        XCTAssertTrue(commands[0].waitForNonExistence(timeout: 5), "发出去以后列表没收起")
        XCTAssertEqual(field.value as? String, "/", "发出去以后输入框里的草稿没了")

        tap(app.buttons["composerCommands"])
        XCTAssertTrue(commands[2].waitForExistence(timeout: 5), "“菜单”按钮没打开命令列表")
    }

    /// 对方正在输入时顶栏状态行换成“正在输入…”。
    func testTopBarShowsTyping() {
        enter(Self.directRoom)
        XCTAssertTrue(app.staticTexts["barActivity"].waitForExistence(timeout: 5), "顶栏没有正在输入\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("direct")
    }

    /// 输入框空着时没有发送键，打了字才出现；“菜单”胶囊一直在。
    func testSendButtonAppearsWithText() {
        enter(Self.directRoom)
        let field = input
        XCTAssertFalse(app.buttons["composerSend"].exists, "输入框空着就有发送键")
        XCTAssertTrue(app.buttons["composerCommands"].exists, "Bot 有命令，输入栏没有“菜单”")
        field.tap()
        waitForFloatingDateToFade()
        shot("composer-empty")
        focus(field)
        field.typeText("Hello")
        XCTAssertTrue(app.buttons["composerSend"].waitForExistence(timeout: 5), "打了字没有发送键")
        XCTAssertTrue(app.buttons["composerCommands"].exists, "打了字“菜单”没了")
        shot("composer-text")
    }

    /// 长按一条消息出系统菜单（ADR-0029），点“回复”进入回复状态。
    func testLongPressShowsMenu() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        message(3).press(forDuration: 1.2)
        let reply = app.buttons["messageReply"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5), "长按没出菜单\n\(app.debugDescription)")
        // 菜单弹出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
        shot("context-menu")
        reply.tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点了“回复”没进入回复状态")
    }

    /// 深色模式下的私聊和群聊：只留截图给人看，断言时间线照常出来。
    func testDarkAppearance() {
        app.terminate()
        XCUIDevice.shared.appearance = .dark
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        app.launch()
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        shot("direct-dark")
        let (first, second) = openAttachSheet()
        shot("attach-tabs-dark")
        _ = selectTwo(first, second)
        shot("attach-selected-dark")
        app.terminate()
        app.launch()
        enter(Self.directRoom)
        openForwardSheet()
        selectTwoRooms()
        shot("forward-sheet-dark")
        app.terminate()
        app.launch()
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group-dark")
    }

    /// 房间顶栏没有“打开面板”的按钮；点头像进 Bot 资料页，从那里打开这个房间的面板（ADR-0052）。
    func testPanelOpensFromBotProfile() {
        enter(Self.directRoom)
        XCTAssertFalse(app.buttons["panelOpen"].exists, "房间顶栏还有打开面板的按钮")
        tap(app.buttons["roomTopBarAvatar"])
        let open = app.buttons["botProfilePanelOpen"]
        for _ in 0..<4 where !open.isHittable {
            app.swipeUp()
        }
        tap(open)
        XCTAssertTrue(app.otherElements["chatPanel"].waitForExistence(timeout: 5), "资料页的按钮没有打开面板\n\(app.debugDescription)")
    }

    /// 回形针打开附件面板：相册的照片网格能点选几张，底下浮着一排切换，私聊里有相册和文件；选了照片那一排换成附言框和
    /// 发送键，关掉回到那一排；写了附言点发送，面板收起。
    func testAttachSheetTabsThenCaptionAndSend() {
        enter(Self.directRoom)
        let (first, second) = openAttachSheet()
        XCTAssertTrue(app.buttons["attachPhotos"].exists, "附件面板底下没有“相册”")
        XCTAssertTrue(app.buttons["attachFiles"].exists, "私聊的附件面板底下没有“文件”")
        XCTAssertFalse(app.buttons["attachSend"].exists, "还没选就有发送键")
        shot("attach-sheet")
        let send = selectTwo(first, second)
        XCTAssertFalse(app.buttons["attachFiles"].exists, "选了照片，那一排切换还在")
        shot("attach-selected")
        tap(app.buttons["attachCancel"])
        XCTAssertTrue(app.buttons["attachFiles"].waitForExistence(timeout: 5), "取消选择没回到那一排切换")
        XCTAssertFalse(first.isSelected || second.isSelected, "取消选择后照片还是选中的")
        first.tap()
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture caption")
        XCTAssertTrue(wait(5) { send.isHittable }, "键盘弹起后发送键点不到")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
    }

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

    /// 附件面板里发出去的照片和附言真到了时间线：附言成了一条消息，附件区空了（ADR-0109 第 9 节）。
    func testAttachSendsPhotoAndCaption() {
        enter(Self.directRoom)
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        XCTAssertTrue(send.isEnabled, "选了照片发送键还是灰的")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture attach caption")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(text(startingWith: "Fixture attach caption").waitForExistence(timeout: 10),
                      "附言没进时间线\n\(app.debugDescription)")
        XCTAssertTrue(element("composerTray").waitForNonExistence(timeout: 10), "发出去了附件区里还留着照片")
        shot("attach-sent")
        darkShot("attach-sent-dark")
    }

    /// 发送被拒：面板照常收起，附言、回复对象和附件区里的照片都留着，让主人重试（ADR-0109 第 9 节）。
    func testRejectedSendKeepsCaptionPhotoAndReply() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_TEST_FAIL_ATTACHMENT"] = "1"
        app.launch()
        enter(Self.directRoom)
        startReply(at: 3)
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture kept caption")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(element("composerTray").waitForExistence(timeout: 5), "被拒以后附件区里的照片没了")
        XCTAssertEqual(input.value as? String, "Fixture kept caption", "被拒以后附言被清掉了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复对象没了")
        shot("attach-rejected")
        darkShot("attach-rejected-dark")
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
        // 转发和附言都排在房间末尾，列表停在锚点时它们不进无障碍树。
        if app.buttons["timelineDown"].exists { tap(app.buttons["timelineDown"]) }
        let arrived = text(startingWith: "Fixture forward comment")
        XCTAssertTrue(arrived.waitForExistence(timeout: 10), "附言没进目标房间\n\(app.debugDescription)")
        // 转发的副本跟在附言下面。列表懒加载，底下的行没画就不在树里：一段段往新的那边拖（dragUp）到它露出来。
        // 原件滚到顶上去了，所以不能按“两条 Fixture message 3”数，按落在附言下面认。
        let copies = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 3"))
        func belowCaption() -> XCUIElement? {
            copies.allElementsBoundByIndex.first { $0.frame.minY > arrived.frame.maxY }
        }
        XCTAssertTrue(
            wait(10) {
                if belowCaption() != nil { return true }
                dragUp()
                return false
            },
            "附言下面没有转发的消息\n\(app.debugDescription)"
        )
        XCTAssertNotNil(belowCaption(), "转发的消息没排在附言下面")
        shot("forwarded-comment")
        darkShot("forwarded-comment-dark")
    }

    /// 照片的字节还在取的时候就显示“正在发送”：面板已经收起，等待期间打的字发完留在输入框里（ADR-0109 第 9 节）。
    func testPhotoBytesWaitShowsSendingAndKeepsTextTypedMeanwhile() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_PHOTO_DELAY_MS"] = "4000"
        app.launch()
        enter(Self.directRoom)
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture caption before")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(element("composerSending").waitForExistence(timeout: 5), "取照片的时候没有“正在发送”")
        let field = input
        focus(field)
        field.typeText("typed while sending")
        XCTAssertTrue(element("composerSending").waitForNonExistence(timeout: 20), "照片发完了“正在发送”还在")
        XCTAssertTrue(((field.value as? String) ?? "").contains("typed while sending"), "等待期间打的字没了")
    }

    /// 旁聊里被拒：草稿和回复对象都留着；提示上的关闭能把它收掉（ADR-0093、ADR-0109 第 9 节）。
    func testSideChatRejectedSendKeepsDraftAndReply() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_FAIL_SEND"] = "1"
        app.launch()
        enter(Self.directRoom)
        // 房间可能停在顶上、也可能已经贴底；贴底时没有“向下”。
        if app.buttons["timelineDown"].exists { tap(app.buttons["timelineDown"]) }
        // 卡片挂在中间偏下的一条消息下面，底下又被附件提示盖住：往上、往下都试，直到它整个露在提示上面。
        let card = app.buttons["sideChatCard"]
        XCTAssertTrue(revealClearOfHint(card), "翻不到露出来的旁聊卡片\n\(app.debugDescription)")
        tap(card)
        // 假数据里旁聊和主对话共用同一份时间线，卡片在盖层里也还在；靠提示认出已经进了旁聊。
        XCTAssertTrue(element("sideChatAttachHint").waitForExistence(timeout: 10), "点了卡片没进旁聊\n\(app.debugDescription)")
        shot("side-chat-hint")
        darkShot("side-chat-hint-dark")
        startReply(at: 3)
        let field = input
        focus(field)
        field.typeText("Fixture side draft")
        tap(app.buttons["composerSend"])
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复对象没了")
        XCTAssertEqual(field.value as? String, "Fixture side draft", "被拒以后草稿被清掉了")
        shot("side-chat-rejected")
        tap(app.buttons["sideChatHintClose"])
        XCTAssertTrue(element("sideChatAttachHint").waitForNonExistence(timeout: 5), "提示上的关闭没把它收掉")
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

    /// 群聊里别人的组尾旁边有头像，私聊里没有。
    func testGroupShowsSenderAvatarAndDirectDoesNot() {
        enter(Self.groupRoom)
        let avatar = app.descendants(matching: .any).matching(identifier: "messageAvatar").firstMatch
        XCTAssertTrue(avatar.waitForExistence(timeout: 5), "群聊里没有发送者头像\n\(app.debugDescription)")
        // 翻到最上面：顶栏下的 AI 那一行占着自己的位置，第一条气泡排在它下面，不被它盖住。
        let aiLine = app.descendants(matching: .any)["roomAiLine"]
        XCTAssertTrue(aiLine.waitForExistence(timeout: 5), "群聊顶栏下没有 AI 那一行\n\(app.debugDescription)")
        let first = message(0)
        reveal(first)
        for _ in 0..<3 { dragDown() }
        XCTAssertTrue(wait(5) { first.exists && first.frame.minY >= aiLine.frame.maxY },
                      "第一条气泡被 AI 那一行盖住\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("group-top")
        // 再进一次，停在最新一条：群聊平常的样子。
        app.terminate()
        app.launch()
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group")
        app.terminate()
        app.launch()
        enter(Self.directRoom)
        XCTAssertFalse(avatar.exists, "私聊里画了发送者头像")
    }

    // MARK: 助手

    /// 点回形针打开附件面板，等相册网格出来，给回前两张照片。
    private func openAttachSheet(file: StaticString = #filePath, line: UInt = #line) -> (XCUIElement, XCUIElement) {
        tap(app.buttons["composerAttach"], file: file, line: line)
        // 第一次打开网格系统会问相册权限。
        // 系统弹窗跟着模拟器的语言走，不认字：三个按钮里中间那个是“允许完全访问”。
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        let first = app.buttons["attachPhoto.0"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        XCTAssertTrue(first.waitForExistence(timeout: 15), "附件面板里没有照片\n\(app.debugDescription)", file: file, line: line)
        // 面板升上来的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
        return (first, app.buttons["attachPhoto.1"])
    }

    /// 选两张照片，等附言框和发送键出来，给回发送键。
    private func selectTwo(_ first: XCUIElement, _ second: XCUIElement) -> XCUIElement {
        first.tap()
        second.tap()
        XCTAssertTrue(wait(5) { first.isSelected && second.isSelected }, "点了的照片没变成选中")
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键\n\(app.debugDescription)")
        XCTAssertTrue(element("attachCaption").exists, "选了照片没有附言框")
        Thread.sleep(forTimeInterval: 0.5)
        return send
    }

    /// 同一处再拍一张深色的，拍完换回浅色。
    private func darkShot(_ name: String) {
        XCUIDevice.shared.appearance = .dark
        waitForFloatingDateToFade()
        shot(name)
        XCUIDevice.shared.appearance = .light
    }

    /// 长按第几条消息点“回复”，等回复条出来。
    private func startReply(at index: Int, file: StaticString = #filePath, line: UInt = #line) {
        reveal(message(index), file: file, line: line)
        waitForFloatingDateToFade()
        message(index).press(forDuration: 1.2)
        let reply = app.buttons["messageReply"]
        guard reply.waitForExistence(timeout: 5) else {
            return XCTFail("长按菜单里没有“回复”\n\(app.debugDescription)", file: file, line: line)
        }
        reply.tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点了“回复”没进入回复状态", file: file, line: line)
    }

    /// 长按一条消息点“转发”，等转发面板出来。
    private func openForwardSheet() {
        waitForFloatingDateToFade()
        message(3).press(forDuration: 1.2)
        let forward = app.buttons["messageForward"]
        XCTAssertTrue(forward.waitForExistence(timeout: 5), "长按菜单里没有“转发”")
        forward.tap()
        XCTAssertTrue(app.textFields["forwardSearch"].waitForExistence(timeout: 5), "转发面板没有搜索框")
    }

    /// 在转发面板里选前两个会话。
    private func selectTwoRooms() {
        let rooms = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "forward-"))
        XCTAssertGreaterThanOrEqual(rooms.count, 2, "转发面板里的会话不到两个")
        rooms.element(boundBy: 0).tap()
        rooms.element(boundBy: 1).tap()
        XCTAssertTrue(app.buttons["forwardSend"].waitForExistence(timeout: 5), "选了会话没有发送键")
        Thread.sleep(forTimeInterval: 0.5)
    }

    /// 按无障碍标识找，不管它是哪种控件（多行的输入框在无障碍树里是 textView）。
    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    /// 浮动日期只在滚动时盖在消息上，停下后淡出，不压着字（ADR-0109 第 4 节）。
    private func waitForFloatingDateToFade(file: StaticString = #filePath, line: UInt = #line) {
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForNonExistence(timeout: 5), "停止滚动后浮动日期没淡出", file: file, line: line)
        // 无障碍树里先没了，淡出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
    }

    private func enter(_ room: String) {
        tap(app.buttons[room])
        // 房间停在顶上还是贴底由落点和贴底决定：等到任意一条假消息就是时间线到了。
        let any = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture ")).firstMatch
        XCTAssertTrue(any.waitForExistence(timeout: 20), "时间线没出来\n\(app.debugDescription)")
        XCTAssertTrue(input.exists, "房间打开了，输入框没出来")
    }

    /// 假房间的第几条消息（正文是 “Fixture message N”）。
    private func message(_ index: Int) -> XCUIElement {
        text(startingWith: "Fixture message \(index)")
    }

    /// 第几条消息自己在屏上、点得到。引用里的摘要和原消息是一样的字，落在引用里的不算。
    private func originalOnScreen(_ index: Int, besides quote: XCUIElement) -> Bool {
        let inQuote = quote.exists ? quote.frame : .null
        return app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message \(index)"))
            .allElementsBoundByIndex
            .contains { inView($0) && !inQuote.contains(CGPoint(x: $0.frame.midX, y: $0.frame.midY)) }
    }

    private func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 输入框。盖层开着时主对话那份也在无障碍树里，取第一份。
    private var input: XCUIElement { app.textViews["Message"].firstMatch }

    /// 列表可见区的上下沿：顶栏那一刻（有人打字时量提示的下沿）到输入框那一刻（上面有提示、附件区时量它们的上沿）。
    private func visibleBounds() -> (top: CGFloat, bottom: CGFloat) {
        let activity = app.staticTexts["barActivity"].firstMatch
        let top = activity.exists ? activity.frame.maxY : app.buttons["roomTopBarAvatar"].firstMatch.frame.maxY
        let cover = [element("sideChatAttachHint"), element("composerTray")].filter { $0.exists }.map { $0.frame.minY }
        let bottom = min(input.frame.minY, cover.min() ?? .infinity)
        return (top, bottom)
    }

    /// 整个落在顶栏和输入框之间。列表垫在半透明的顶栏下面，那里的元素 `isHittable` 也是真，点下去点到的是顶栏；
    /// 输入框上面的附件区和旁聊提示一样盖住列表。
    private func inView(_ node: XCUIElement) -> Bool {
        guard node.exists, node.isHittable else { return false }
        let (top, bottom) = visibleBounds()
        return node.frame.minY > top && node.frame.maxY < bottom
    }

    /// 翻到露在附件提示上面的位置：卡片在提示底下就往上挪，在提示上面更远处就往下找。
    private func revealClearOfHint(_ node: XCUIElement) -> Bool {
        for _ in 0..<60 {
            guard node.exists else {
                dragDown()
                continue
            }
            let (top, bottom) = visibleBounds()
            if node.frame.minY > top, node.frame.maxY < bottom, node.isHittable { return true }
            if node.frame.maxY <= top {
                dragDown()
            } else {
                dragUp()
            }
        }
        return false
    }

    /// 从底部一段一段往回翻，直到这个元素整个在可见区里。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<60 {
            if inView(element) { return }
            dragDown()
        }
        XCTFail("往回翻了 60 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 点一下输入框，等它真拿到键盘焦点；没拿到就重试。焦点还在上一个框里时 typeText 会打进那个框。
    private func focus(_ field: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<5 {
            field.tap()
            // XCUITest 没有公开的焦点 API，`hasKeyboardFocus` 走 KVC 读。
            if (field.value(forKey: "hasKeyboardFocus") as? Bool) == true { return }
        }
        XCTFail("输入框一直没拿到键盘焦点\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往新的一头挪一小段：从提示上面起手，挪 110 点，不会把整屏翻过头。
    private func dragUp() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.42)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 往回翻一段。慢拖、松手前停一下：不带惯性，不会一下翻过头。
    private func dragDown() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
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

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
