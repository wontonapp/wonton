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

    /// 输入框开头敲“/”弹出命令列表，按前缀过滤；点一条直接发出去，输入框清空；“菜单”按钮打开同一张列表。
    func testSlashOpensCommandMenuAndTapSends() {
        enter(Self.directRoom)
        let field = app.textViews["Message"]
        let commands = ["help", "model", "new"].map { app.buttons["command.\($0)"] }
        XCTAssertFalse(commands[0].exists, "没敲“/”就有命令列表")
        field.tap()
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
        XCTAssertFalse(app.buttons["composerSend"].exists, "发出去以后输入框没清空")

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
        let field = app.textViews["Message"]
        XCTAssertFalse(app.buttons["composerSend"].exists, "输入框空着就有发送键")
        XCTAssertTrue(app.buttons["composerCommands"].exists, "Bot 有命令，输入栏没有“菜单”")
        field.tap()
        waitForFloatingDateToFade()
        shot("composer-empty")
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
        tap(app.buttons["botProfilePanelOpen"])
        XCTAssertTrue(app.otherElements["chatPanel"].waitForExistence(timeout: 5), "资料页的按钮没有打开面板\n\(app.debugDescription)")
    }

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

    /// 浮动日期只在滚动时盖在消息上，停下后淡出，不压着字（ADR-0109 第 4 节）。
    private func waitForFloatingDateToFade(file: StaticString = #filePath, line: UInt = #line) {
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForNonExistence(timeout: 5), "停止滚动后浮动日期没淡出", file: file, line: line)
        // 无障碍树里先没了，淡出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
    }

    private func enter(_ room: String) {
        tap(app.buttons[room])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
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

    /// 整个落在顶栏和输入框之间。列表垫在半透明的顶栏下面，那里的元素 `isHittable` 也是真，点下去点到的是顶栏。
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
        XCTFail("往回翻了 60 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
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
