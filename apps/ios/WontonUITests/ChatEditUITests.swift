import XCTest

/// 编辑自己发出去的消息（ADR-0109 第 10 节）：菜单里的“编辑”、输入框上方的编辑条、取消恢复草稿、确认换正文。
/// 不要后端：用 `WONTON_UI_FAKE` 的假房间（`ui_fake_room.rs` 给条目，`ui_fake_timeline.rs` 响应编辑操作）。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ChatEditUITests
final class ChatEditUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let room = "chatRoom.!ui-fake:home.example"
    /// 自己发的普通文字（`ui_fake_room.rs` 第 15 条）。
    private static let ownIndex = 15
    /// Echo 发的（第 14 条）：菜单里没有“编辑”。
    private static let foreignIndex = 14
    private static let original = "Fixture message 15"
    private static let draft = "draft typed before editing"
    private static let replacement = "Fixture edited text"

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 每条要翻时间线、敲字、拍浅深两张；慢的时候翻一条要几分钟，给满 xcodebuild 的上限。
        executionTimeAllowance = 600
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    /// 长按自己的消息点“编辑”：输入框换成原文，上面出现编辑条，确认键换成勾；取消后编辑条收起、草稿回来、正文没变。
    func testEditShowsBarAndCancelRestoresDraft() {
        enter()
        goToBottom()
        revealBackwards(message(Self.ownIndex))
        let field = input
        focus(field)
        field.typeText(Self.draft)
        XCTAssertEqual(field.value as? String, Self.draft, "进编辑前的草稿没打进去")

        openEdit(Self.ownIndex)
        let bar = element("composerEditBar")
        XCTAssertTrue(bar.waitForExistence(timeout: 5), "点了“编辑”没出编辑条\n\(app.debugDescription)")
        XCTAssertEqual(field.value as? String, Self.original, "输入框没换成原文")
        XCTAssertEqual(element("editSummary").label, Self.original, "编辑条没写原文摘要")
        let confirm = app.buttons["composerSend"]
        XCTAssertTrue(confirm.exists, "编辑时没有确认键")
        XCTAssertEqual(confirm.label, "Edit", "发送键没换成确认")
        waitForFloatingDateToFade()
        shot("edit-bar")
        // 深色：先退出编辑态，在深色里重新进去，免得玻璃停在浅色的旧帧；键盘开着会一直是浅色，拍之前收掉。
        setAppearance(.dark)
        forceTap(app.buttons["editCancel"])
        XCTAssertTrue(bar.waitForNonExistence(timeout: 5), "切深色时没退出编辑态")
        openEdit(Self.ownIndex)
        hideKeyboard()
        waitForFloatingDateToFade()
        shot("edit-bar-dark")
        setAppearance(.light)

        forceTap(app.buttons["editCancel"])
        XCTAssertTrue(bar.waitForNonExistence(timeout: 5), "取消了编辑条还在\n\(app.debugDescription)")
        XCTAssertTrue(wait(5) { (field.value as? String) == Self.draft },
                      "取消以后输入框没回到编辑前的草稿：\(field.value ?? "nil")")
        let body = message(Self.ownIndex)
        if !body.exists { reveal(body) }
        XCTAssertTrue(body.exists && body.label.hasPrefix(Self.original), "取消以后消息正文变了")
        XCTAssertFalse(app.descendants(matching: .any)["messageEdited"].exists, "取消以后标了“已编辑”")
        XCTAssertEqual(confirm.label, "Send", "退出编辑以后确认键没换回发送")
    }

    /// 别人的消息：菜单里有回复、没有“编辑”。
    func testForeignMessageMenuHasNoEdit() {
        enter()
        goToBottom()
        let message = message(Self.foreignIndex)
        revealBackwards(message)
        waitForFloatingDateToFade()
        message.press(forDuration: 1.2)
        XCTAssertTrue(app.buttons["messageReply"].waitForExistence(timeout: 5),
                      "长按没出菜单\n\(app.debugDescription)")
        XCTAssertFalse(app.buttons["messageEdit"].exists, "别人的消息菜单里有“编辑”")
        app.buttons["messageReply"].tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点“回复”没进回复状态")
    }

    /// 改掉正文再确认：编辑条收起、这一条换成新正文并标“已编辑”，不新增一条；改过的字留在输入框当草稿。
    func testConfirmReplacesTextAndShowsEdited() {
        enter()
        goToBottom()
        openEdit(Self.ownIndex)
        let field = input
        XCTAssertTrue(wait(5) { (field.value as? String) == Self.original }, "输入框没换成原文")
        // 点在右端，光标落在原文末尾，删干净再打新的。
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.5)).tap()
        field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: Self.original.count + 4))
        XCTAssertEqual(field.value as? String, "", "原文没删干净")
        field.typeText(Self.replacement)

        forceTap(app.buttons["composerSend"])
        XCTAssertTrue(element("composerEditBar").waitForNonExistence(timeout: 5), "确认以后编辑条没收起")
        // 编辑途中打过字，原草稿就不盖回来了（同核心）：新正文留在输入框里，发送键换回“Send”。
        let confirm = app.buttons["composerSend"]
        XCTAssertTrue(wait(5) { (field.value as? String) == Self.replacement }, "确认以后输入框里的新正文没了")
        XCTAssertTrue(wait(5) { confirm.label == "Send" }, "确认以后发送键没换回发送")
        let edited = text(startingWith: Self.replacement)
        shot("after-confirm")
        // 编辑条收起会让列表挪一点，这一条可能正好被挤出屏幕；往回翻着找。
        revealBackwards(edited)
        XCTAssertFalse(text(startingWith: Self.original).exists, "旧正文还在\n\(app.debugDescription)")
        XCTAssertTrue(wait(5) { self.inView(self.element("messageEdited")) }, "确认以后没标“已编辑”")

        waitForFloatingDateToFade()
        shot("edit-sent")
        darkShot("edit-sent-dark")
    }

    // MARK: 助手

    /// 长按一条自己的消息，点菜单里的“编辑”，等编辑条出来。
    private func openEdit(_ index: Int, file: StaticString = #filePath, line: UInt = #line) {
        let message = message(index)
        revealBackwards(message, file: file, line: line)
        waitForFloatingDateToFade()
        message.press(forDuration: 1.2)
        let edit = app.buttons["messageEdit"]
        guard edit.waitForExistence(timeout: 5) else {
            return XCTFail("自己的消息菜单里没有“编辑”\n\(app.debugDescription)", file: file, line: line)
        }
        edit.tap()
        XCTAssertTrue(element("composerEditBar").waitForExistence(timeout: 5), "点了“编辑”没进编辑态", file: file, line: line)
    }

    /// 房间开在靠前的落点上；要改的那条在底下，先点“向下”翻到最新，再往回找。
    private func goToBottom() {
        let down = app.buttons["timelineDown"]
        if down.waitForExistence(timeout: 5) {
            tap(down)
            _ = down.waitForNonExistence(timeout: 10)
        }
        _ = wait(5) { self.input.isHittable }
    }

    private func enter(file: StaticString = #filePath, line: UInt = #line) {
        tap(app.buttons[Self.room], file: file, line: line)
        let any = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture ")).firstMatch
        XCTAssertTrue(any.waitForExistence(timeout: 20), "时间线没出来\n\(app.debugDescription)", file: file, line: line)
        XCTAssertTrue(input.exists, "房间打开了，输入框没出来", file: file, line: line)
    }

    /// 假房间的第几条消息（正文是 “Fixture message N”）。
    private func message(_ index: Int) -> XCUIElement {
        text(startingWith: "Fixture message \(index)")
    }

    private func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 按无障碍标识找，不管它是哪种控件。
    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    private var input: XCUIElement { app.textViews["Message"].firstMatch }

    /// 列表可见区的上下沿：顶栏那一刻到输入框那一刻。
    private func visibleBounds() -> (top: CGFloat, bottom: CGFloat) {
        let activity = app.staticTexts["barActivity"].firstMatch
        let top = activity.exists ? activity.frame.maxY : app.buttons["roomTopBarAvatar"].firstMatch.frame.maxY
        let cover = [element("sideChatAttachHint"), element("composerTray")].filter { $0.exists }.map { $0.frame.minY }
        let bottom = min(input.frame.minY, cover.min() ?? .infinity)
        return (top, bottom)
    }

    private func inView(_ node: XCUIElement) -> Bool {
        guard node.exists, node.isHittable else { return false }
        let (top, bottom) = visibleBounds()
        return node.frame.minY > top && node.frame.maxY < bottom
    }

    private func waitForFloatingDateToFade(file: StaticString = #filePath, line: UInt = #line) {
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForNonExistence(timeout: 5), "停止滚动后浮动日期没淡出", file: file, line: line)
        // 无障碍树里先没了，淡出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
    }

    /// 翻到元素：画出来了的按位置往那头翻，没画出来先往旧消息那头找，到头再反过来。
    private func revealBackwards(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<60 {
            if element.isHittable { return }
            let forward = element.exists && element.frame.minY > input.frame.minY
            drag(backwards: !forward)
        }
        XCTFail("翻了 60 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    private func drag(backwards: Bool) {
        let from = backwards ? CGVector(dx: 0.5, dy: 0.15) : CGVector(dx: 0.5, dy: 0.9)
        let to = backwards ? CGVector(dx: 0.5, dy: 0.9) : CGVector(dx: 0.5, dy: 0.15)
        app.coordinate(withNormalizedOffset: from)
            .press(forDuration: 0.01, thenDragTo: app.coordinate(withNormalizedOffset: to),
                   withVelocity: .default, thenHoldForDuration: 0)
    }

    /// 大片翻到能点元素：先朝新消息翻，到底再反过来。目标在靠近底部，几下就到。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for backwards in [false, true] {
            for _ in 0..<30 {
                if element.isHittable { return }
                drag(backwards: backwards)
            }
        }
        XCTFail("两头各翻 30 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    private func focus(_ field: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<5 {
            field.tap()
            if (field.value(forKey: "hasKeyboardFocus") as? Bool) == true { return }
        }
        XCTFail("输入框一直没拿到键盘焦点\n\(app.debugDescription)", file: file, line: line)
    }

    /// 不靠 `isHittable` 直接点元素的中心：小按钮被同层的透明记号挡着也算得到。
    private func forceTap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        guard wait(10, until: { element.exists }) else {
            return XCTFail("找不到 \(element)\n\(app.debugDescription)", file: file, line: line)
        }
        let frame = element.frame
        app.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: frame.midX, dy: frame.midY))
            .tap()
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

    /// 轻拖一下收起键盘（列表是交互式收起）。
    private func hideKeyboard() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.45, dy: 0.42))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.45, dy: 0.62)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
        _ = app.keyboards.firstMatch.waitForNonExistence(timeout: 5)
    }

    /// 同一处再拍一张深色的，拍完换回浅色。
    private func darkShot(_ name: String) {
        setAppearance(.dark)
        waitForFloatingDateToFade()
        shot(name)
        setAppearance(.light)
    }

    /// 换外观并等画面真的变了：刚建好的模拟器上 `appearance` 偶尔要好几秒才生效，只等固定时间会拍到旧颜色。
    private func setAppearance(_ appearance: XCUIDevice.Appearance) {
        let dark = appearance == .dark
        // 最多等 5 秒；还没变就随它，拍下来人眼看。
        XCUIDevice.shared.appearance = appearance
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if screenIsDark() == dark { return }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    /// 左上角几个点的平均亮度接近黑就是深色。
    private func screenIsDark() -> Bool {
        guard let cg = XCUIScreen.main.screenshot().image.cgImage,
              let data = cg.dataProvider?.data,
              let bytes = CFDataGetBytePtr(data) else { return false }
        let bpp = max(cg.bitsPerPixel / 8, 3)
        var sum = 0
        for (x, y) in [(100, 300), (600, 300), (1100, 300), (600, 2000)] {
            guard x < cg.width, y < cg.height else { continue }
            let offset = y * cg.bytesPerRow + x * bpp
            sum += (Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])) / 3
        }
        return sum / 4 < 100
    }
}
