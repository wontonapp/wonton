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
        XCTAssertTrue(waitFull(), "再往下推没有到第 2 级")
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
        XCTAssertTrue(waitFull(), "页面滚到顶后接着拉没到第 2 级")
        shot("06-page-full")
    }

    /// 整页的面板和插件页共用胶囊：胶囊一直在，切到后台再回来也在，不能困在页面里（盲测 P0，ADR-0095）。
    /// 用最大的辅助字号和深色跑：盲测里这两样下退出也被挤掉或看不见过。
    func testCapsuleStaysAfterBackground() throws {
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
        XCTAssertTrue(waitFull(), "没到第 2 级")
        // 以前的胶囊 3 秒没碰就藏起来，等过这段时间它还得在。
        Thread.sleep(forTimeInterval: 5)
        XCTAssertTrue(capsule.isHittable, "胶囊自己藏起来了")
        shot("21-still-there")

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
        pullStrip(.upward)
        XCTAssertTrue(waitFull(), "在露出的聊天条上推没有到整页")
        shot("12-room-full")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "胶囊退出后面板还开着")
    }

    func testHomePullRequiresDeliberateGesture() throws {
        try launch()
        XCTAssertNotNil(botRoom(timeout: 30), "Real chat list has not finished loading")
        shot("polish-home-before-pulls")
        let panel = app.otherElements["chatPanel"]
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        drag(start, dy: 20, velocity: .fast)
        XCTAssertTrue(waitGone(panel), "Tiny fast home pull opened the panel")
        drag(start, dy: 64, velocity: .slow)
        XCTAssertTrue(waitGone(panel), "Short slow home pull opened the panel")
        drag(start, dy: 120, velocity: .slow)
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Deliberate slow home pull did not open")
        shot("polish-home-slow-peek")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.94)).tap()
        XCTAssertTrue(waitGone(panel))
        drag(start, dy: 64, velocity: XCUIGestureVelocity(rawValue: 1_200))
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Qualified fast home pull did not open")
    }

    func testRoomPullRequiresDeliberateGesture() throws {
        try launch()
        XCTAssertNotNil(enterBotRoom())
        let panel = app.otherElements["chatPanel"]
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.65))
        drag(start, dy: -20, velocity: .fast)
        XCTAssertTrue(waitGone(panel), "Tiny fast room pull opened the panel")
        drag(start, dy: -64, velocity: .slow)
        XCTAssertTrue(waitGone(panel), "Short slow room pull opened the panel")
        drag(start, dy: -120, velocity: .slow)
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Deliberate slow room pull did not open")
        shot("polish-room-slow-peek")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
        XCTAssertTrue(waitGone(panel))
        drag(start, dy: -64, velocity: XCUIGestureVelocity(rawValue: 1_200))
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Qualified fast room pull did not open")
    }

    func testSidebarRequiresDeliberateEdgeSwipe() throws {
        try launch()
        let collapse = app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Collapse sidebar", "收起侧边栏", "收起側邊欄", "サイドバーを閉じる"]
        )).firstMatch
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5))
            .withOffset(CGVector(dx: 4, dy: 0))
        let tiny = start.withOffset(CGVector(dx: 20, dy: 0))
        start.press(forDuration: 0.05, thenDragTo: tiny, withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(waitGone(collapse), "Tiny fast edge swipe opened the sidebar")
        let short = start.withOffset(CGVector(dx: 48, dy: 0))
        start.press(forDuration: 0.05, thenDragTo: short, withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(waitGone(collapse), "Short slow edge swipe opened the sidebar")
        let deliberate = start.withOffset(CGVector(dx: 80, dy: 0))
        start.press(forDuration: 0.05, thenDragTo: deliberate, withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(collapse.waitForExistence(timeout: 5), "Slow edge swipe did not open the sidebar")
        shot("polish-sidebar-deliberate")
        collapse.tap()
        XCTAssertTrue(waitGone(collapse))
        let flick = start.withOffset(CGVector(dx: 48, dy: 0))
        start.press(forDuration: 0.05, thenDragTo: flick,
                    withVelocity: XCUIGestureVelocity(rawValue: 1_200), thenHoldForDuration: 0)
        XCTAssertTrue(collapse.waitForExistence(timeout: 5), "Qualified edge flick did not open the sidebar")
        collapse.tap()
        XCTAssertTrue(waitGone(collapse))
        let button = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(button.isHittable, "Accessible sidebar button was lost")
        button.tap()
        XCTAssertTrue(collapse.waitForExistence(timeout: 5))
    }

    func testChatMaterialUnderlapAndPanelSeam() throws {
        try launch()
        guard let field = enterBotRoom() else { return XCTFail("Bot room unavailable") }
        field.tap()
        field.typeText(String(repeating: "Material underlap sample: long message with several lines. ", count: 30))
        tapAnyButton(["Send", "发送", "傳送", "送信"], timeout: 5, required: true)
        field.typeText("Multiline draft\nSecond line\nThird line")
        shot("polish-composer-keyboard-multiline")
        let list = app.collectionViews.firstMatch
        XCTAssertTrue(list.exists)
        XCTAssertGreaterThan(list.frame.maxY, field.frame.minY, "Timeline does not extend behind composer material")
        list.swipeDown(velocity: .slow)
        shot("polish-top-and-composer-under-scrolled-message")
        goToBottom()
        shot("polish-last-message-above-composer")
        pullUpInRoom()
        XCTAssertTrue(app.otherElements["chatPanel"].waitForExistence(timeout: 5))
        if app.buttons["panelAdd"].exists {
            app.buttons["panelAdd"].tap()
            let models = app.buttons.matching(NSPredicate(
                format: "label BEGINSWITH %@", "Models"
            )).firstMatch
            XCTAssertTrue(models.waitForExistence(timeout: 10), "Installed Models page unavailable")
            models.tap()
            tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        }
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "Plugin panel did not render")
        shot("polish-composer-panel-seam")
    }

    private func drag(_ start: XCUICoordinate, dy delta: CGFloat, velocity: XCUIGestureVelocity) {
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: delta)),
                    withVelocity: velocity, thenHoldForDuration: 0)
    }

    /// The list renders behind the composer, but its inset keeps the last message above the field
    /// while a multiline draft and the keyboard change the unobscured viewport.
    func testLastMessageClearsComposer() throws {
        try launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        guard let field = enterBotRoom() else {
            return XCTFail("没进 Bot 的房间（开发栈默认装 Hermes，或先装回声 Bot）")
        }
        field.tap()
        field.typeText(String(repeating: "long reply text ", count: 30))
        tapAnyButton(["Send", "发送"], timeout: 5, required: true)
        Thread.sleep(forTimeInterval: 2)
        shot("29-after-send")
        let sent = frames(list: app.collectionViews.firstMatch, field: field)
        attach("after-send \(sent) \(scrollState())")

        field.typeText(String(repeating: "draft ", count: 40))
        Thread.sleep(forTimeInterval: 2)
        shot("30-composer-grown")

        let list = app.collectionViews.firstMatch
        XCTAssertTrue(list.exists, "房间里没有消息列表")
        let now = frames(list: list, field: field)
        let state = scrollState()
        attach("after-draft \(now) \(state)")
        // 贴底看 App 自己的位置：List 的内容高度比真实内容矮一段，按内容外框比较在长行上必败（见 PR 说明）。
        XCTAssertTrue(state.contains("pin=true") && isAtBottom(state), "发完以后列表没贴底：\(now) \(state)")
        XCTAssertLessThanOrEqual(visibleBottom(state), Double(field.frame.minY + 1),
                                 "Last-message viewport extends under the composer: \(now)")
    }
}

extension ChatPanelUITests {

    // MARK: - 摆台

    private func launch(arguments: [String] = []) throws {
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += arguments
        try UITestBackend.passPairingCode(to: app)
        app.launch()
        if app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] != nil {
            XCTAssertTrue(waitPaired(timeout: 90), "配对没完成（配对码过期、用过或连不上开发栈）")
        }
        // 落点二选一：引导页、已配对直进的聊天列表。
        // 不等“新建聊天”：mini 实测聊天列表加载中它不一定出现，侧边栏按钮和列表本体更稳。
        let arrived = waitAnyButton(
            ["Apps", "应用", "Sidebar", "侧边栏", "側邊欄", "サイドバー",
             "Skip", "跳过", "Next", "下一步", "Done", "完成"],
            timeout: 60
        ) != nil || app.scrollViews.firstMatch.waitForExistence(timeout: 2)
        XCTAssertTrue(arrived, "既没到聊天列表，也没到引导页")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
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

    private enum Direction { case upward, down }

    /// 从面板网页的顶边附近往下拉：页面滚到顶后接着拉。页面里面起手的归网页，不拖面板（ADR-0052）。
    private func pullPageDown() {
        // 主面板的网页框从屏幕顶上开始，上面一截在状态栏下面：起手点放在状态栏下沿往下的起手区里。
        // UI 测试读不到安全区；走屏用的机型状态栏高 47–62pt，起手区 48pt。靠左起手，不落在胶囊上。
        let top = max(app.webViews.firstMatch.frame.minY + 12, 70)
        let inside = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 48, dy: top))
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
        case .upward: high.press(forDuration: 0.05, thenDragTo: low, withVelocity: .slow, thenHoldForDuration: 0)
        }
    }

    /// 进 Bot 的房间，返回输入框。新配对的设备走完引导直接落在主 Bot 的房间里，这时不用再从列表点进去。
    private func enterBotRoom() -> XCUIElement? {
        skipKeyHandoff()
        if let field = composer(timeout: 3) {
            goToBottom()
            return field
        }
        botRoom(timeout: 30)?.tap()
        guard let field = composer(timeout: 10) else { return nil }
        goToBottom()
        return field
    }

    /// 房间落在已读位置（开发栈账号常攒着未读），不是最新一条。落点是首批时间线到了才摆的，比输入框出现晚，
    /// 摆的时候还会把列表拉回已读位置，所以要等按钮出现（最多 10 秒），按下去、等它消失，再确认没有回来。
    private func goToBottom() {
        var tapped = false
        for _ in 0..<3 {
            guard let button = bottomButton(timeout: tapped ? 2 : 10) else { break }
            tapped = true
            button.tap()
            _ = waitGone(button, timeout: 5)
        }
    }

    /// 停在别处时才有的“回到底部”，数字悬在图标上方，标签会带新消息条数。
    private func bottomButton(timeout: TimeInterval) -> XCUIElement? {
        let labels = ["Back to bottom", "回到底部", "回到底部", "一番下へ戻る"]
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            for label in labels {
                let button = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label)).firstMatch
                if button.exists { return button }
            }
            if Date() >= deadline { return nil }
            _ = app.wait(for: .runningForeground, timeout: 0.3)
        }
    }

    /// 量一次列表：内容的底边是不是还在列表可见区里。
    private struct Layout: CustomStringConvertible {
        let contentBottom: CGFloat
        let listBottom: CGFloat
        let fieldTop: CGFloat

        /// 停在别处时才有“回到底部”按钮：它出现说明 App 自己也知道没到底。
        let back: Bool

        var description: String { "content=\(contentBottom) list=\(listBottom) field=\(fieldTop) back=\(back)" }
    }

    private func frames(list: XCUIElement, field: XCUIElement) -> Layout {
        // 列表的第一个子元素是整片内容（含屏幕外的部分）。
        Layout(contentBottom: list.children(matching: .other).firstMatch.frame.maxY,
               listBottom: list.frame.maxY, fieldTop: field.frame.minY, back: bottomButton(timeout: 0) != nil)
    }

    /// App 里 `BottomPin` 的自述：贴底没贴底、真实位置和底部差多少。
    private func scrollState() -> String {
        app.staticTexts["roomScroll"].label
    }

    /// `snapshot` 里的 y 是不是到位（`bottom` 是 App 算出来的底部）。
    private func isAtBottom(_ state: String) -> Bool {
        let pairs = state.split(separator: " ").compactMap { part -> (String, Double)? in
            let pair = part.split(separator: "=", maxSplits: 1)
            guard pair.count == 2, let value = Double(pair[1]) else { return nil }
            return (String(pair[0]), value)
        }
        let values = Dictionary(uniqueKeysWithValues: pairs)
        guard let offsetY = values["y"], let bottom = values["bottom"] else { return false }
        return offsetY >= bottom - 2
    }

    private func visibleBottom(_ state: String) -> Double {
        let part = state.split(separator: " ").first { $0.hasPrefix("visibleBottom=") }
        return part.flatMap { Double($0.dropFirst("visibleBottom=".count)) } ?? .infinity
    }

    private func attach(_ text: String) {
        let attachment = XCTAttachment(string: text)
        attachment.name = text
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// 新配上的设备先整屏盖着加设备的密钥交接页“在另一台设备上继续”，落进房间后才出来也有。
    /// 这里不测交接：选“手边没有别的设备”，再从它打开的聊天备份页退回列表。
    private func skipKeyHandoff() {
        let labels = ["I don't have my other device", "手边没有别的设备"]
        guard let button = waitAnyButton(labels, timeout: 10) else { return }
        button.tap()
        tapAnyButton(["Back", "返回"], timeout: 5)
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
        let list = app.collectionViews.firstMatch
        let field = composer(timeout: 1)
        let upper = max(app.frame.minY + 150, list.frame.minY + 24)
        let lower = min(list.frame.maxY, field?.frame.minY ?? list.frame.maxY) - 24
        XCTAssertGreaterThan(lower - upper, 96, "Not enough unobscured timeline for a deliberate pull")
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let start = origin.withOffset(CGVector(dx: app.frame.midX, dy: lower))
        let end = origin.withOffset(CGVector(dx: app.frame.midX, dy: upper))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 面板到了第 2 级（整页）。第 1 级也有胶囊（iOS 没有系统返回键），不能拿胶囊在不在当整页的信号。
    private func waitFull(timeout: TimeInterval = 5) -> Bool {
        let panel = app.otherElements["chatPanel"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if panel.exists, panel.value as? String == "full" { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return false
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
            if UITestBackend.skipKeyHandoff(app) { continue }
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
