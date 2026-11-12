import XCTest

/// 聊天页下拉面板（#33，ADR-0052）的走屏：主聊天页拉到第 1 级、推进第 2 级、胶囊退出、空面板加一页网址。
///
/// 要连家里的服务器（开发栈）：外部传 `PAIRING_CODE`（`deploy/dev/core/dev.sh pair <地址>` 打出的配对码），
/// App 启动时交给核心配对（`WontonApp` 的 `WONTON_DEV_PAIRING_CODE`），没有就跳过。
final class ChatPanelUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testPullPanelLevelsAndCapsule() throws {
        try launch()
        shot("01-list")
        let panel = app.otherElements["chatPanel"]
        XCTAssertFalse(panel.exists, "面板一开始就是开的")

        pull(.down)
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "列表到顶后往下拉没有面板")
        shot("02-peek")
        XCTAssertFalse(capsule.exists, "第 1 级就有胶囊")
        XCTAssertTrue(app.buttons["panelAdd"].exists, "空面板没有添加页面的加号")

        pullStrip(.down)
        XCTAssertTrue(capsule.waitForExistence(timeout: 5), "再往下推没有到第 2 级")
        shot("03-full")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "胶囊退出后面板还开着")
        shot("04-closed")

        pull(.down)
        XCTAssertTrue(app.buttons["panelAdd"].waitForExistence(timeout: 5), "空面板加号没出来")
        app.buttons["panelAdd"].tap()
        let field = app.textFields["https://"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "编辑面板没有网址输入框")
        field.tap()
        field.typeText("https://example.com")
        tapAnyButton(["Add URL", "添加网址", "新增網址", "URL を追加"], timeout: 5, required: true)
        tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        XCTAssertFalse(app.buttons["panelAdd"].waitForExistence(timeout: 2), "加了网址后还是空面板")
        shot("05-page")

        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 10), "面板网页没加载出来")
        pullPageDown()
        XCTAssertTrue(capsule.waitForExistence(timeout: 5), "页面滚到顶后接着拉没到第 2 级")
        shot("06-page-full")
    }

    /// 整页的面板和全屏页共用胶囊：胶囊藏起来后切到后台再回来，出口要重新露出来，不能困在页面里（盲测 P0）。
    /// 用最大的辅助字号和深色跑：盲测里这两样下退出也被挤掉或看不见过。
    func testCapsuleReturnsAfterBackground() throws {
        continueAfterFailure = false
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        try launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        pull(.down)
        XCTAssertTrue(app.buttons["panelAdd"].waitForExistence(timeout: 10), "空面板加号没出来")
        app.buttons["panelAdd"].tap()
        let field = app.textFields["https://"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "编辑面板没有网址输入框")
        field.tap()
        field.typeText("https://example.com")
        tapAnyButton(["Add URL", "添加网址", "新增網址", "URL を追加"], timeout: 5, required: true)
        tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "面板网页没加载出来")
        pullPageDown()
        XCTAssertTrue(capsule.waitForExistence(timeout: 5), "没到第 2 级")
        XCTAssertTrue(waitGone(capsule, timeout: 8), "胶囊没自己藏起来")
        shot("21-hidden")

        pullPageDown()
        XCTAssertTrue(capsule.waitForExistence(timeout: 3), "在网页上往下拖叫不出胶囊")
        XCTAssertTrue(waitGone(capsule, timeout: 8), "胶囊没自己藏起来")

        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来")
        XCTAssertTrue(capsule.waitForExistence(timeout: 3), "切回来后整页上没有出口")
        XCTAssertTrue(capsule.isHittable && app.frame.contains(capsule.frame), "出口被挤出屏幕或点不到")
        shot("22-after-background")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "胶囊退出后面板还开着")
        shot("23-closed")
    }

    func testRoomPullFromBottom() throws {
        try launch()
        XCTAssertNotNil(enterBotRoom(), "没进 Bot 的房间（开发栈默认装 Hermes，或先装回声 Bot）")

        let panel = app.otherElements["chatPanel"]
        pullUpInRoom()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "到底后往上推没有面板")
        shot("11-room-peek")
        pullStrip(.up)
        XCTAssertTrue(capsule.waitForExistence(timeout: 5), "在露出的聊天条上推没有到整页")
        shot("12-room-full")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "胶囊退出后面板还开着")
    }

    /// 长消息的最后一段不被输入框盖住：大字号下发一条长消息，再把输入框打到多行、键盘弹着，列表仍贴底，
    /// 内容的底边在列表里、列表的底边在输入框上面。不按正文找那一行：开发栈里本机常解不开自己刚发的消息。
    func testLastMessageClearsComposer() throws {
        try launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        guard let field = enterBotRoom() else {
            return XCTFail("没进 Bot 的房间（开发栈默认装 Hermes，或先装回声 Bot）")
        }
        field.tap()
        field.typeText(String(repeating: "long reply text ", count: 30))
        tapAnyButton(["Send", "发送"], timeout: 5, required: true)
        field.typeText(String(repeating: "draft ", count: 40))
        _ = app.wait(for: .runningForeground, timeout: 3)
        shot("30-composer-grown")

        let list = app.collectionViews.firstMatch
        XCTAssertTrue(list.exists, "房间里没有消息列表")
        // 列表的第一个子元素是整片内容（含屏幕外的部分）。
        let content = list.children(matching: .other).firstMatch.frame
        XCTAssertLessThanOrEqual(content.maxY, list.frame.maxY + 2, "发完以后列表没贴底，最后一条在可见区下面")
        XCTAssertLessThanOrEqual(list.frame.maxY, field.frame.minY + 1, "列表的底边被输入框盖住")
    }

    // MARK: - 摆台

    private func launch(arguments: [String] = []) throws {
        app = XCUIApplication()
        app.launchArguments += arguments
        if let code = try UITestBackend.codes().first {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
        app.launch()
        if app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] != nil {
            XCTAssertTrue(waitPaired(timeout: 90), "配对没完成（配对码过期、用过或连不上开发栈）")
        }
        // 落点三选一：引导页、标签栏、已配对直进的聊天列表（这时底栏可能整个收起）。
        // 不等“新建聊天”：mini 实测聊天列表加载中它不一定出现，侧边栏按钮和列表本体更稳。
        let arrived = waitAnyButton(
            ["Apps", "应用", "Sidebar", "侧边栏", "側邊欄", "サイドバー",
             "Skip", "跳过", "Next", "下一步", "Done", "完成"],
            timeout: 60
        ) != nil || app.scrollViews.firstMatch.waitForExistence(timeout: 2)
        XCTAssertTrue(arrived, "既没到标签栏、聊天列表，也没到引导页")
        passOnboarding()
        // 配上以后只有聊天一个标签时底栏收起，没有「聊天」按钮；没配对时底栏在，要点进聊天页。
        tapAnyButton(["Chats", "聊天"], timeout: 2)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 20), "没到聊天页")
    }

    /// 配对页的输入框不在、又已经出了页面（聊天列表或引导）才算配上；出现拒绝提示就不用再等。
    private func waitPaired(timeout: TimeInterval) -> Bool {
        let anyElement = app.descendants(matching: .any)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if anyElement["pairing-refusal"].exists { return false }
            let rendered = app.scrollViews.firstMatch.exists || app.buttons["Skip"].exists || app.buttons["跳过"].exists
            if rendered && !anyElement["pairing-code"].exists { return true }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return false
    }

    private enum Direction { case up, down }

    /// 列表/页面中间往下拉：页面滚到顶后接着拉。
    private func pullPageDown() {
        let inside = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        let outward = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85))
        inside.press(forDuration: 0.05, thenDragTo: outward, withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 第 1 级露出的那一条聊天页：点它回去，往外推换级。
    private func pullStrip(_ direction: Direction) {
        let inside = direction == .down
            ? app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
            : app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1))
        let outward = direction == .down
            ? app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.99))
            : app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02))
        inside.press(forDuration: 0.05, thenDragTo: outward, withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 从列表里往上下拖。用慢速拖动，不让惯性把列表甩走。
    private func pull(_ direction: Direction) {
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 10), "找不到聊天列表")
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.28))
        let high = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.92))
        switch direction {
        case .down: low.press(forDuration: 0.05, thenDragTo: high, withVelocity: .slow, thenHoldForDuration: 0)
        case .up: high.press(forDuration: 0.05, thenDragTo: low, withVelocity: .slow, thenHoldForDuration: 0)
        }
    }

    /// 进 Bot 的房间，返回输入框。新配对的设备走完引导直接落在主 Bot 的房间里，这时不用再从列表点进去。
    private func enterBotRoom() -> XCUIElement? {
        if let field = composer(timeout: 3) { return field }
        botRoom(timeout: 30)?.tap()
        return composer(timeout: 10)
    }

    /// 列表里 Bot 的房间行。按名字找：列表顶上还可能有整行宽的提示（如恢复聊天备份），按位置挑会点错。
    private func botRoom(timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for name in ["Hermes", "回声", "Echo"] {
                let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", name)).firstMatch
                if row.exists, row.isHittable { return row }
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return nil
    }

    /// 输入框：聊天列表本身也是滚动视图，等它出现才算进了房间。
    private func composer(timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(min(timeout, 10))
        while Date() < deadline {
            for label in ["Message", "消息", "訊息", "メッセージ"] {
                for view in app.textViews.allElementsBoundByIndex where view.label == label && view.exists {
                    return view
                }
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return nil
    }

    /// 房间里的手指从不挡输入框的位置往上推。
    private func pullUpInRoom() {
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72))
        let high = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        low.press(forDuration: 0.05, thenDragTo: high, withVelocity: .slow, thenHoldForDuration: 0)
    }

    private var capsule: XCUIElement {
        app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Close", "关闭", "關閉", "閉じる"]
        )).firstMatch
    }

    /// 先点收尾的，再点下一步，最后才跳过：最后一步的“跳过”是灰的，“进入聊天”才走得出去。
    private static let onboardingButtons = [
        "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成",
        "Next", "下一步", "Skip", "跳过"
    ]

    /// 主人在这台核心上走过引导时不再弹，所以引导按钮没出现也算走完。
    private func passOnboarding() {
        for _ in 0..<12 {
            guard let button = waitAnyButton(Self.onboardingButtons, timeout: 3) else { return }
            button.tap()
        }
    }

    @discardableResult
    private func waitAnyButton(_ labels: [String], timeout: TimeInterval, required: Bool = false) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(min(timeout, 10))
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                // 引导最后一页的「跳过」还在、只是禁用：不看 isEnabled 会一直点它，走不到「完成」。
                if element.exists, element.isEnabled, element.isHittable { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        if required { XCTFail("没等到按钮：\(labels)") }
        return nil
    }

    private func tapAnyButton(_ labels: [String], timeout: TimeInterval, required: Bool = false) {
        guard let button = waitAnyButton(labels, timeout: timeout, required: required) else { return }
        button.tap()
    }

    private func waitGone(_ element: XCUIElement, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !element.exists { return true }
            _ = app.wait(for: .runningForeground, timeout: 0.3)
        }
        return !element.exists
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
