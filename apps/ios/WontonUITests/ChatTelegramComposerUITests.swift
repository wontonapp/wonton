import XCTest

/// 命令菜单、“向下”给菜单让位、顶栏正在输入、发送键出现、长按菜单。
final class ChatTelegramComposerUITests: ChatTelegramUITestSupport {
    /// 命令菜单占着右下角时“向下”让开，收起菜单再回来。
    func testDownButtonHidesWhileCommandMenuOpen() {
        enter(Self.directRoom)
        let down = app.buttons["timelineDown"]
        tap(down)
        dragDown()
        XCTAssertTrue(down.waitForExistence(timeout: 5), "离开底部后没有“向下”\n\(app.debugDescription)")
        let field = app.textViews["Message"]
        field.tap()
        field.typeText("/")
        XCTAssertTrue(app.buttons["command.help"].waitForExistence(timeout: 5), "敲“/”没开命令菜单")
        XCTAssertTrue(down.waitForNonExistence(timeout: 5), "菜单开着还压着“向下”\n\(app.debugDescription)")
        field.typeText(XCUIKeyboardKey.delete.rawValue)
        XCTAssertTrue(down.waitForExistence(timeout: 5), "收起菜单后“向下”没回来\n\(app.debugDescription)")
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
}
