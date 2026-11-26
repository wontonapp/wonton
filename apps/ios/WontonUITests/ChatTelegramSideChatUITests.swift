import XCTest

/// 旁聊：从顶栏新建、进入和返回、照片上限、多选、说明归属、发送被拒。
final class ChatTelegramSideChatUITests: ChatTelegramUITestSupport {
    /// 顶栏开新旁聊：写第一句发出去，面板收完盖层才上来，第一句落在这间旁聊里。
    func testNewSideChatFromTopBar() {
        enter(Self.directRoom)
        tap(app.buttons["sideChatNew"])
        let field = app.textFields["sideChatFirstMessage"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "顶栏没打开新旁聊的面板\n\(app.debugDescription)")
        field.tap()
        field.typeText("Hello side")
        tap(app.buttons["sideChatStart"])
        XCTAssertTrue(app.buttons["barBack"].waitForExistence(timeout: 10), "发完第一句没进旁聊\n\(app.debugDescription)")
        XCTAssertTrue(text(startingWith: "Hello side").waitForExistence(timeout: 10), "第一句没发进旁聊")
        XCTAssertFalse(app.textViews["Message"].firstMatch.isHittable, "旁聊没盖住主对话")
        shot("side-chat-new")
    }

    /// 点主对话里的旁聊卡片进去：上面盖的是这间旁聊的对话，不是退回聊天列表；返回键收掉旁聊、主对话回来。
    func testSideChatOpens() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let last = text(startingWith: "Fixture side 3")
        XCTAssertTrue(last.exists, "旁聊里没有最后一条回复")
        XCTAssertTrue(app.buttons["barBack"].exists, "旁聊顶栏没有返回")
        shot("side-chat")
        tap(app.buttons["barBack"])
        // 盖层收到一半时的画面：内容要被 `onDismiss` 留到转场跑完，不能这一下就抽走。
        shot("side-chat-back")
        XCTAssertTrue(last.waitForNonExistence(timeout: 10), "返回以后旁聊的内容还在")
        XCTAssertFalse(app.buttons["barBack"].exists, "返回以后旁聊的顶栏还在")
        XCTAssertTrue(wait(10) { app.textViews["Message"].firstMatch.isHittable }, "返回以后主对话的输入栏没回来")
        XCTAssertTrue(app.buttons["sideChatCard.$ui-fake-5"].exists, "返回以后主对话没回来")
    }

    /// 旁聊的附件面板只有相册：没有“文件”那一项，照片照常出得来。
    func testSideChatAttachShowsPhotosOnly() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        _ = openSideAttach()
        XCTAssertFalse(app.buttons["attachFiles"].exists, "旁聊的附件面板里还有“文件”")
        shot("side-chat-attach")
    }

    /// 旁聊的相册点得动：照片不是灰的，选一张、点“完成”，照片真的发进旁聊。
    func testSideChatPhotoGridUsable() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let first = openSideAttach()
        XCTAssertTrue(first.isEnabled, "旁聊里照片点不了，上限还是 0 个")
        first.tap()
        XCTAssertTrue(first.isSelected, "点了的照片没变成选中")
        sendPickedPhotos()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了“完成”面板没收起")
        XCTAssertTrue(waitForCount(photoEchoes, 1, timeout: 10), "选的照片没发进旁聊")
        XCTAssertFalse(app.buttons["dialogOk"].exists, "发照片弹了错误")
    }

    /// 旁聊一次能选好几张：三张都在一张面板里选上，不用开三次。
    func testSideChatPicksSeveralPhotosAtOnce() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<3).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[2].waitForExistence(timeout: 15), "旁聊的附件面板里没有三张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上三张")
        XCTAssertTrue(app.buttons["attachSend"].exists, "选了三张没有“完成”")
        shot("side-chat-multi")
        sendPickedPhotos()
        XCTAssertTrue(waitForCount(photoEchoes, 3, timeout: 20), "三张照片没有都发进旁聊")
        XCTAssertFalse(app.buttons["dialogOk"].exists, "发照片弹了错误")
    }

    /// 旁聊的说明只跟第一张照片：发两张时第一张带说明、第二张不带，草稿清掉。
    func testSideChatCaptionGoesOnlyWithFirstPhoto() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let field = input
        focus(field)
        field.typeText("Fixture first caption")
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<2).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[1].waitForExistence(timeout: 15), "旁聊的附件面板里没有两张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上两张")
        sendPickedPhotos()
        XCTAssertTrue(
            text(startingWith: "Fixture first caption").waitForExistence(timeout: 20),
            "第一张没带上说明\n\(app.debugDescription)"
        )
        XCTAssertTrue(text(startingWith: "photo").waitForExistence(timeout: 20), "第二张没发出去\n\(app.debugDescription)")
        let captions = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture first caption"))
        XCTAssertEqual(captions.count, 1, "说明跟了不止一张\n\(app.debugDescription)")
        XCTAssertEqual(field.value as? String, "", "说明随照片发出去了，草稿没清")
    }

    /// 第一张被拒：说明留在草稿里，第二张照发但不带说明。
    func testSideChatRefusedFirstPhotoKeepsCaptionForLater() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_FAIL_SEND"] = "1"
        app.launch()
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let field = input
        focus(field)
        field.typeText("Fixture kept caption")
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<2).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[1].waitForExistence(timeout: 15), "旁聊的附件面板里没有两张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上两张")
        sendPickedPhotos()
        XCTAssertTrue(text(startingWith: "photo").waitForExistence(timeout: 20), "第一张被拒以后第二张没发出去\n\(app.debugDescription)")
        XCTAssertFalse(text(startingWith: "Fixture kept caption").exists, "说明跟了第二张，本该留在草稿里\n\(app.debugDescription)")
        XCTAssertEqual(field.value as? String, "Fixture kept caption", "第一张被拒后草稿没了")
    }

    /// 旁聊里被拒：草稿和回复对象都留着；提示上的关闭能把它收掉（ADR-0093、ADR-0109 第 9 节）。
    func testSideChatRejectedSendKeepsDraftAndReply() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_FAIL_SEND"] = "1"
        UITestBackend.launch(app)
        enter(Self.directRoom)
        // 卡片就挂在进房落点那条：没画出来往上挪一两段，不用横穿整条时间线；
        // 底下可能被附件提示盖住，交给 revealClearOfHint 挪到提示上面。
        let card = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sideChatCard.")).firstMatch
        for _ in 0..<3 where !card.exists { dragDown() }
        XCTAssertTrue(revealClearOfHint(card), "翻不到露出来的旁聊卡片\n\(app.debugDescription)")
        tap(card)
        // 假数据里旁聊和主对话共用同一份时间线，卡片在盖层里也还在；靠提示认出已经进了旁聊。
        XCTAssertTrue(element("sideChatAttachHint").waitForExistence(timeout: 10), "点了卡片没进旁聊\n\(app.debugDescription)")
        shot("side-chat-hint")
        darkShot("side-chat-hint-dark")
        // 旁聊里是它自己的假时间线（`Fixture side N`），回复紧挨着落点的那条。
        startReply(on: text(startingWith: "Fixture side 1"))
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

    // MARK: 助手

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

    /// 点主对话里的旁聊卡片进去。翻到卡片后按当时的位置现算坐标：画布里的元素缓存下来的 frame 会过期。
    /// 面板里的“完成”：把选好的照片发出去。
    private func sendPickedPhotos(file: StaticString = #filePath, line: UInt = #line) {
        tap(app.buttons["attachSend"], file: file, line: line)
    }

    /// 等到查询里至少 [count] 个元素出现。
    private func waitForCount(_ query: XCUIElementQuery, _ count: Int, timeout: TimeInterval) -> Bool {
        wait(timeout) { query.count >= count }
    }

    /// 假核心给每张发出去的照片回一条“photo”。
    private var photoEchoes: XCUIElementQuery {
        app.staticTexts.matching(NSPredicate(format: "label == %@", "photo"))
    }
}
