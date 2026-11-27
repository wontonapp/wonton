import XCTest

/// 附件面板：相册和文件切换、附言、发送、被拒保留、取照片期间打的字。
final class ChatTelegramAttachUITests: ChatTelegramUITestSupport {
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
        UITestBackend.launch(app)
        enter(Self.directRoom)
        // 旁聊里是另一份假时间线（Fixture side N），回复落点旁边那条。
        startReply(on: text(startingWith: "Fixture side 1"))
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

    /// 照片的字节还在取的时候就显示“正在发送”：面板已经收起，等待期间打的字发完留在输入框里（ADR-0109 第 9 节）。
    func testPhotoBytesWaitShowsSendingAndKeepsTextTypedMeanwhile() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_PHOTO_DELAY_MS"] = "4000"
        UITestBackend.launch(app)
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
}
