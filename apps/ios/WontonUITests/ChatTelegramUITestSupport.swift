import XCTest

/// 聊天页照 Telegram 的几样交互（ADR-0109）：左滑回复、点引用跳到原消息、“向下”先回到跳之前那一条、
/// “/”命令菜单、消息下的选项按钮、顶栏的正在输入、群聊里组尾的头像。
/// 不要后端：用 `WONTON_UI_FAKE` 的假房间（核心的 `ui_fake_room.rs` 给条目，`ui_fake_timeline.rs` 响应操作）。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test \
///         -only-testing:WontonUITests/ChatTelegramReplyQuoteUITests \
///         -only-testing:WontonUITests/ChatTelegramComposerUITests \
///         -only-testing:WontonUITests/ChatTelegramSideChatUITests \
///         -only-testing:WontonUITests/ChatTelegramAttachUITests \
///         -only-testing:WontonUITests/ChatTelegramForwardUITests \
///         -only-testing:WontonUITests/ChatTelegramLayoutUITests
class ChatTelegramUITestSupport: XCTestCase {
    var app: XCUIApplication!

    static let directRoom = "chatRoom.!ui-fake:home.example"
    static let groupRoom = "chatRoom.!ui-fake-call:home.example"
    static let notesRoom = "chatRoom.!ui-fake-notes:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 顶栏叠放那条要翻遍整条时间线，默认 360 秒会被 kill 在半路、连断言里的树都拿不到。
        executionTimeAllowance = 900
        app = XCUIApplication()
        // 不恢复上一次的场景状态：房间记着上次看到哪一条（SceneStorage），上一条用例翻到别处会带到这一条来。
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        // 深色轮次由跑测脚本经 `TEST_RUNNER_WONTON_DARK=1` 传进来；XCUITest 每轮开场会把设备外观改回上一次的样子，
        // 只在外面 `simctl ui` 切会被盖回去。
        if ProcessInfo.processInfo.environment["WONTON_DARK"] == "1" {
            XCUIDevice.shared.appearance = .dark
        }
        UITestBackend.launch(app)
    }

    // MARK: 助手

    /// 点回形针打开附件面板，等相册网格出来，给回前两张照片。
    func openAttachSheet(file: StaticString = #filePath, line: UInt = #line) -> (XCUIElement, XCUIElement) {
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
    func selectTwo(_ first: XCUIElement, _ second: XCUIElement) -> XCUIElement {
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
    func darkShot(_ name: String) {
        XCUIDevice.shared.appearance = .dark
        waitForFloatingDateToFade()
        shot(name)
        XCUIDevice.shared.appearance = .light
    }

    /// 长按第几条消息点“回复”，等回复条出来。
    func startReply(at index: Int, file: StaticString = #filePath, line: UInt = #line) {
        startReply(on: message(index), file: file, line: line)
    }

    /// 长按指到的消息点“回复”，等回复条出来。
    func startReply(on element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        reveal(element, file: file, line: line)
        waitForFloatingDateToFade()
        element.press(forDuration: 1.2)
        let reply = app.buttons["messageReply"]
        guard reply.waitForExistence(timeout: 5) else {
            return XCTFail("长按菜单里没有“回复”\n\(app.debugDescription)", file: file, line: line)
        }
        reply.tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点了“回复”没进入回复状态", file: file, line: line)
    }

    /// 长按一条消息点“转发”，等转发面板出来。转第 4 条：第 3 条是照片，假数据只转发文字（`text_of` 拿不到照片的正文）。
    func openForwardSheet() {
        waitForFloatingDateToFade()
        message(4).press(forDuration: 1.2)
        let forward = app.buttons["messageForward"]
        XCTAssertTrue(forward.waitForExistence(timeout: 5), "长按菜单里没有“转发”")
        forward.tap()
        XCTAssertTrue(app.textFields["forwardSearch"].waitForExistence(timeout: 5), "转发面板没有搜索框")
    }

    /// 在转发面板里选前两个会话。
    func selectTwoRooms() {
        let rooms = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "forward-"))
        XCTAssertGreaterThanOrEqual(rooms.count, 2, "转发面板里的会话不到两个")
        rooms.element(boundBy: 0).tap()
        rooms.element(boundBy: 1).tap()
        XCTAssertTrue(app.buttons["forwardSend"].waitForExistence(timeout: 5), "选了会话没有发送键")
        Thread.sleep(forTimeInterval: 0.5)
    }

    /// 按无障碍标识找，不管它是哪种控件（多行的输入框在无障碍树里是 textView）。
    func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    /// 浮动日期只在滚动时盖在消息上，停下后淡出，不压着字（ADR-0109 第 4 节）。
    func waitForFloatingDateToFade(file: StaticString = #filePath, line: UInt = #line) {
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForNonExistence(timeout: 5), "停止滚动后浮动日期没淡出", file: file, line: line)
        // 无障碍树里先没了，淡出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
    }

    func enter(_ room: String) {
        tap(app.buttons[room])
        // 房间停在顶上还是贴底由落点和贴底决定：等到任意一条假消息就是时间线到了。
        let any = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture ")).firstMatch
        XCTAssertTrue(any.waitForExistence(timeout: 20), "时间线没出来\n\(app.debugDescription)")
        XCTAssertTrue(input.exists, "房间打开了，输入框没出来")
    }

    /// 假房间的第几条消息（正文是 “Fixture message N”）。
    func message(_ index: Int) -> XCUIElement {
        text(startingWith: "Fixture message \(index)")
    }

    func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 输入框。盖层开着时主对话那份也在无障碍树里，取第一份。
    var input: XCUIElement { app.textViews["Message"].firstMatch }

    /// 列表可见区的上下沿：顶栏那一刻（有人打字时量提示的下沿）到输入框那一刻（上面有提示、附件区时量它们的上沿）。
    func visibleBounds() -> (top: CGFloat, bottom: CGFloat) {
        let activity = app.staticTexts["barActivity"].firstMatch
        let top = activity.exists ? activity.frame.maxY : app.buttons["roomTopBarAvatar"].firstMatch.frame.maxY
        let cover = [element("sideChatAttachHint"), element("composerTray")].filter { $0.exists }.map { $0.frame.minY }
        let bottom = min(input.frame.minY, cover.min() ?? .infinity)
        return (top, bottom)
    }

    /// 整个落在顶栏和输入框之间。列表垫在半透明的顶栏下面，那里的元素 `isHittable` 也是真，点下去点到的是顶栏；
    /// 输入框上面的附件区和旁聊提示一样盖住列表。
    func inView(_ node: XCUIElement) -> Bool {
        guard node.exists, node.isHittable else { return false }
        let (top, bottom) = visibleBounds()
        return node.frame.minY > top && node.frame.maxY < bottom
    }

    /// 一段一段翻到元素整个在可见区里。默认往回翻；元素已经画在输入框下面时朝新消息翻，
    /// 房间开在落点上，紧贴着落点下面的东西（旁聊卡）不用先翻到最上面再回来。
    func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for step in 0..<200 {
            if inView(element) { return }
            if element.exists {
                // 已经画出来、但在输入框下面（朝新消息的一侧）就往下翻，其余往回翻。
                // 旁聊页开着时树里有两个 “Message”，走 `input` 取第一份。
                if element.frame.minY > input.frame.minY { dragUp() } else { dragDown() }
            } else if step < 12 {
                // 没画出来就按锚点猜：房间开在落点上，落点下面紧贴的卡（旁聊卡）要先往回翻。
                dragDown()
            } else {
                // 猜的这头翻到头了还没有：换一头。翻到头时列表回弹，两边都试满才判失败。
                if step % 10 < 5 { dragDown() } else { dragUp() }
            }
        }
        XCTFail("翻了 200 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 点一下输入框，等它真拿到键盘焦点；没拿到就重试。焦点还在上一个框里时 typeText 会打进那个框。
    func focus(_ field: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<5 {
            field.tap()
            // XCUITest 没有公开的焦点 API，`hasKeyboardFocus` 走 KVC 读。
            if (field.value(forKey: "hasKeyboardFocus") as? Bool) == true { return }
        }
        XCTFail("输入框一直没拿到键盘焦点\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往新的一头挪一小段：从提示上面起手，挪 110 点，不会把整屏翻过头。
    func dragUp() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.42)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 大片地翻到能点元素：房间开在进来的那条附近，旁聊卡片就在两三条之后，先往新消息那边翻，到底再反过来。
    /// 一次拖大半屏，按能不能点判断，比逐条消息找快。
    func scrollTo(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for backwards in [false, true] {
            for _ in 0..<30 {
                if element.isHittable { return }
                let from = backwards ? CGVector(dx: 0.5, dy: 0.15) : CGVector(dx: 0.5, dy: 0.9)
                let to = backwards ? CGVector(dx: 0.5, dy: 0.9) : CGVector(dx: 0.5, dy: 0.15)
                app.coordinate(withNormalizedOffset: from)
                    .press(forDuration: 0.01, thenDragTo: app.coordinate(withNormalizedOffset: to),
                           withVelocity: .default, thenHoldForDuration: 0)
            }
        }
        XCTFail("两头各翻 30 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往回翻一段。慢拖、松手前停一下：不带惯性，不会一下翻过头。
    func dragDown() {
        // 起点放在下半屏：顶栏下叠着插件横幅时，0.35 会压在那张卡片上，拖不动列表。
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 旁聊里打开附件面板，等相册网格出来，返回第一张照片。主对话被盖住时它的输入栏还在无障碍树里，要上面那间的按钮。
    func openSideAttach(file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let attaches = app.buttons.matching(identifier: "composerAttach").allElementsBoundByIndex
        tap(attaches.first(where: { $0.isHittable }) ?? attaches[0], file: file, line: line)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        let first = app.buttons["attachPhoto.0"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        guard first.waitForExistence(timeout: 15) else {
            XCTFail("旁聊的附件面板里没有照片\n\(app.debugDescription)", file: file, line: line)
            return first
        }
        return first
    }

    func openSideChat(file: StaticString = #filePath, line: UInt = #line) {
        let card = app.buttons["sideChatCard.$ui-fake-5"]
        scrollTo(card, file: file, line: line)
        let f = card.frame
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: f.midX, dy: f.midY)).tap()
        guard wait(10, until: { text(startingWith: "Fixture side ").exists }) else {
            return XCTFail("点了旁聊卡片没进旁聊\n\(app.debugDescription)", file: file, line: line)
        }
    }

    func tap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        guard wait(20, until: { element.exists && element.isHittable }) else {
            return XCTFail("找不到或点不到 \(element)\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
