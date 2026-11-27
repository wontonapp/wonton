import XCTest

/// 顶栏叠放、选项按钮、深色模式、宽表格横滚、最大字号、群聊头像。
final class ChatTelegramLayoutUITests: ChatTelegramUITestSupport {
    /// 顶栏下面照 Telegram 的叠放：插件横幅在最上，旁聊标题在它下面。
    func testPluginBannerSitsAboveSideChatTitle() {
        enter(Self.notesRoom)
        // 卡片挂在落点那条（`SIDE_CHAT_TOP_INDEX`）：等落点摆稳（落点滚动出的日期胶囊淡掉）它就在眼前；
        // 再不行往上翻一两段，不指望“向下”按钮，也不用横穿整条时间线。
        waitForFloatingDateToFade()
        // 卡片标识带根事件 ID（`sideChatCard.<root>`），按前缀取一张。
        let card = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sideChatCard.")).firstMatch
        for _ in 0..<3 where !card.exists { dragDown() }
        XCTAssertTrue(card.waitForExistence(timeout: 10), "找不到旁聊卡\n\(app.debugDescription)")
        reveal(card)
        tap(card)
        // 盖着的那份主对话被 accessibilityHidden，横幅只留一份；先到先取仍稳妥。
        let banner = app.descendants(matching: .any)["pluginStatusBanner"].firstMatch
        let header = app.descendants(matching: .any)["sideChatHeader"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 5), "旁聊页顶栏下没有插件横幅\n\(app.debugDescription)")
        XCTAssertTrue(header.waitForExistence(timeout: 5), "旁聊页顶栏下没有旁聊标题\n\(app.debugDescription)")
        XCTAssertLessThan(banner.frame.maxY, header.frame.minY,
                          "插件横幅不在旁聊标题上面\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("side-chat-top")
        // 旁聊页自带一层导航栈：这是它那一层的根，左边缘右滑不把整个房间划走（ADR-0093）。
        let edge = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
        edge.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.1)
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertTrue(header.exists, "右滑把房间划走了\n\(app.debugDescription)")
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

    /// 深色模式下的私聊、旁聊和群聊：只留截图给人看，断言时间线照常出来。
    func testDarkAppearance() {
        app.terminate()
        XCUIDevice.shared.appearance = .dark
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        UITestBackend.launch(app)
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        shot("direct-dark")
        let (first, second) = openAttachSheet()
        shot("attach-tabs-dark")
        _ = selectTwo(first, second)
        shot("attach-selected-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        openSideChat()
        shot("side-chat-dark")
        let photo = openSideAttach()
        shot("side-chat-attach-dark")
        for index in 0..<3 {
            let cell = app.buttons["attachPhoto.\(index)"]
            XCTAssertTrue(cell.waitForExistence(timeout: 10), "相册网格没铺出第 \(index) 张")
            cell.tap()
        }
        XCTAssertTrue(wait(5) { photo.isSelected }, "深色下旁聊的相册选不上照片")
        shot("side-chat-multi-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        openForwardSheet()
        selectTwoRooms()
        shot("forward-sheet-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group-dark")
    }

    /// 宽表格上横拖不抢回复：从右往左拖是表格自己滚，回复条不出现；从左往右拖才进回复。
    func testTableScrollsSidewaysWithoutStartingReply() {
        enter(Self.directRoom)
        let cell = text(startingWith: "Fixture cell 0-0")
        tap(app.buttons["timelineDown"])
        reveal(cell)
        waitForFloatingDateToFade()
        shot("table-before")
        for _ in 0..<3 {
            let start = cell.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.5))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 160, dy: 0)),
                        withVelocity: .default, thenHoldForDuration: 0.1)
            XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "表格上往右拖没有进回复\n\(app.debugDescription)")
            app.buttons["replyCancel"].tap()
        }
        shot("table-replied")
        for _ in 0..<3 {
            let before = cell.frame.minX
            let start = cell.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: 0.5))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -120, dy: 0)),
                        withVelocity: .default, thenHoldForDuration: 0.1)
            XCTAssertFalse(app.buttons["replyCancel"].exists, "表格上往左拖进了回复\n\(app.debugDescription)")
            XCTAssertLessThan(cell.frame.minX, before - 20, "表格没跟着往左滚")
        }
        shot("table-scrolled")
    }

    /// 最大字号下，房间名的胶囊和带两个选项的按钮还在框里：文字不截断、按钮不裁字。
    func testLargestTextKeepsBarAndChoicesWhole() {
        app.terminate()
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        // 深色轮次由跑测脚本经 `TEST_RUNNER_WONTON_DARK=1` 传进来；XCUITest 每轮开场会把设备外观改回上一次的样子，
        // 只在外面 `simctl ui` 切会被盖回去。
        if ProcessInfo.processInfo.environment["WONTON_DARK"] == "1" {
            XCUIDevice.shared.appearance = .dark
        }
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        let title = app.staticTexts["Family"]
        XCTAssertTrue(title.waitForExistence(timeout: 10), "顶栏没有房间名\n\(app.debugDescription)")
        shot("large-text-bar")
        XCTAssertEqual(title.label, "Family", "房间名被改写了")
        XCTAssertLessThan(title.frame.maxX, app.windows.element(boundBy: 0).frame.maxX, "房间名顶出了屏幕")
        tap(app.buttons["timelineDown"])
        let approve = app.buttons["messageChoice.✅"]
        reveal(approve)
        waitForFloatingDateToFade()
        shot("large-text-choices")
        for (button, label) in [(approve, "approve once"), (app.buttons["messageChoice.❌"], "deny")] {
            XCTAssertTrue(button.exists, "\(label) 的选项按钮不见了")
            let inside = app.staticTexts[label].firstMatch
            XCTAssertTrue(inside.exists, "\(label) 的文字不见了")
            XCTAssertLessThanOrEqual(inside.frame.maxY, button.frame.maxY + 0.5, "\(label) 的字被按钮裁掉")
            XCTAssertLessThanOrEqual(inside.frame.maxX, button.frame.maxX + 0.5, "\(label) 的字横着出框")
        }
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
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        XCTAssertFalse(avatar.exists, "私聊里画了发送者头像")
    }
}
