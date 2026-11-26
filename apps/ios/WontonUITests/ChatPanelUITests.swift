import XCTest
import ObjectiveC

/// 聊天面板按钮（ADR-0052）的走屏：空面板第 1 级、有页面第 2 级、展开后换级与胶囊退出。
///
/// 默认用 `WONTON_UI_FAKE` 的假数据，夜跑带了配对码也一样：房间里的用例不依赖开发栈装没装 Bot。
/// 要真插件页的用例连开发栈，外部传 `PAIRING_CODE`（`deploy/dev/core/dev.sh pair <地址>` 打出的配对码），
/// App 启动时交给核心配对（`WontonApp` 的 `WONTON_DEV_PAIRING_CODE`）；没给码时这几条跳过。
final class ChatPanelUITests: XCTestCase { // swiftlint:disable:this type_body_length
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testPanelButtonLevelsAndCapsule() throws {
        try launch(needsStack: true)
        shot("01-list")
        let panel = app.otherElements["chatPanel"]
        XCTAssertFalse(panel.exists, "面板一开始就是开的")

        openPanel()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "聊天顶栏按钮没有打开面板")
        XCTAssertEqual(panel.value as? String, "peek", "空面板按钮没有进入第 1 级")
        shot("02-peek")
        XCTAssertFalse(capsule.exists, "空面板就有胶囊")
        XCTAssertTrue(app.buttons["panelAdd"].exists, "空面板没有添加页面的加号")

        // 空面板最多到第 1 级（ADR-0052）：先加一页网址，才推得到整页。
        app.buttons["panelAdd"].tap()
        let field = app.textFields["https://"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "编辑面板没有网址输入框")
        field.tap()
        field.typeText("https://example.com")
        tapAnyButton(["Add URL", "添加网址", "新增網址", "URL を追加"], timeout: 5, required: true)
        tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        XCTAssertFalse(app.buttons["panelAdd"].waitForExistence(timeout: 2), "加了网址后还是空面板")
        shot("03-page")

        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 10), "面板网页没加载出来")
        pullPageDown()
        XCTAssertTrue(waitFull(), "页面滚到顶后接着拉没到第 2 级")
        shot("04-page-full")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule) && waitGone(panel), "胶囊退出后面板还开着")
        shot("05-closed")

        openPanel()
        XCTAssertTrue(waitFull(), "有页面时按钮没有直接打开整页")
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 10), "重新打开后面板网页不在")
        XCTAssertTrue(capsule.exists, "重新打开后没有胶囊")
        shot("06-reopened")
    }

    /// 整页的面板和插件页共用胶囊：胶囊一直在，切到后台再回来也在，不能困在页面里（盲测 P0，ADR-0095）。
    /// 用最大的辅助字号和深色跑：盲测里这两样下退出也被挤掉或看不见过。
    func testCapsuleStaysAfterBackground() throws {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        try launch(
            arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"],
            needsStack: true
        )
        openPanel()
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

    func testRoomPanelButtonAndExpandedGestures() throws {
        try launch()
        XCTAssertNotNil(enterBotRoom(), "没进 Bot 的房间（假数据里是 Echo，开发栈里是 Hermes）")

        let panel = app.otherElements["chatPanel"]
        openPanel()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "房间顶栏按钮没有打开面板")
        if panel.value as? String == "peek" {
            addPageIfEmpty()
            pullStrip(.upward)
        }
        XCTAssertTrue(waitFull(), "有页面的房间面板没有进入整页")
        shot("12-room-full")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "胶囊退出后面板还开着")
    }

    func testHomeScrollingNeverOpensPanel() throws {
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
        XCTAssertTrue(waitGone(panel), "Long home scroll opened the panel")
        drag(start, dy: 64, velocity: XCUIGestureVelocity(rawValue: 1_200))
        XCTAssertTrue(waitGone(panel), "Fast home scroll opened the panel")
        openPanel()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Home panel button did not open")
    }

    /// 房间里往下往上滚都不打开面板，只有 Bot 资料页里的入口打开。（#556 报的是这条：改用资料页入口以后它换了名字，
    /// issue 和夜跑里还写着旧的 `testRoomPullFromBottom`。）
    func testRoomScrollingNeverOpensPanel() throws {
        try launch()
        XCTAssertNotNil(enterBotRoom())
        let panel = app.otherElements["chatPanel"]
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.65))
        drag(start, dy: -20, velocity: .fast)
        XCTAssertTrue(waitGone(panel), "Tiny fast room pull opened the panel")
        drag(start, dy: -64, velocity: .slow)
        XCTAssertTrue(waitGone(panel), "Short slow room pull opened the panel")
        drag(start, dy: -120, velocity: .slow)
        XCTAssertTrue(waitGone(panel), "Long room scroll opened the panel")
        drag(start, dy: -64, velocity: XCUIGestureVelocity(rawValue: 1_200))
        XCTAssertTrue(waitGone(panel), "Fast room scroll opened the panel")
        openPanel()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "Room panel button did not open")
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
        // 收起动画还没完时按钮可点，但这一下会被吞；等它落定再点。
        Thread.sleep(forTimeInterval: 0.6)
        button.tap()
        XCTAssertTrue(collapse.waitForExistence(timeout: 5))
    }

    func testChatMaterialUnderlapAndPanelSeam() throws {
        try launch(needsStack: true)
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
        openPanel()
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
            return XCTFail("没进 Bot 的房间（假数据里是 Echo，开发栈里是 Hermes）")
        }
        field.tap()
        field.typeText(String(repeating: "long reply text ", count: 30))
        let send = app.buttons["composerSend"]
        XCTAssertTrue(send.waitForExistence(timeout: 5), "打了字没有发送键")
        send.tap()
        Thread.sleep(forTimeInterval: 2)
        shot("29-after-send")
        let sent = frames(list: app.collectionViews.firstMatch, field: field)
        attach("after-send \(sent) \(scrollState())")

        // 点发送会收起键盘，再打草稿得先点回输入框；不点回来，下面量的是没键盘的可见区。
        field.tap()
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

    /// 回到底部按钮（ADR-0071）：翻进历史就出现，只有箭头时命中区不小于 48 的点按门槛；
    /// 后台消息把数字顶上去时也是同一块命中区，键盘弹起时抬过输入栏，点一下平滑回到底部、按钮消失。
    func testScrollToBottomButtonMeetsTarget() throws {
        try launch()
        guard let field = enterFakeRoom() else {
            return XCTFail("没进假房间")
        }
        // 进房落点摆好、回到底部以后按钮不该在；它要真在，下面“拖出来”的断言会被一开始就挂着的按钮骗过。
        XCTAssertTrue(noBottomButton(timeout: 2), "停在底部时就有按钮：\(buttonLabels()) \(scrollState())")
        // 在时间线上拖：叠层里那层记 ov= 的 Color.clear 得放行命中，否则拖不动、按钮永远出不来。
        let before = scrollOffsetY()
        XCTAssertTrue(scrollUpInRoom(), "翻走以后没有回到底部按钮：\(buttonLabels()) \(scrollState())")
        XCTAssertLessThan(scrollOffsetY(), before, "拖了列表没动：\(scrollState())")
        guard let button = anyBottomButton(timeout: 10) else {
            return XCTFail("翻走以后没有回到底部按钮：\(buttonLabels()) \(scrollState())\n\(app.debugDescription)")
        }
        XCTAssertGreaterThanOrEqual(button.frame.width, 47.5, "命中区宽 \(button.frame.width) 不到 48")
        XCTAssertGreaterThanOrEqual(button.frame.height, 47.5, "命中区高 \(button.frame.height) 不到 48")
        shot("31-scroll-button")
        guard let counted = countedBottomButton(3, timeout: 10) else {
            return XCTFail("翻走以后数字没到 3：\(buttonLabels())")
        }
        XCTAssertGreaterThanOrEqual(counted.frame.height, 47.5, "带数字时命中区高 \(counted.frame.height) 不到 48")
        shot("32-scroll-button-count")
        XCTAssertTrue(scrollState().contains("vp=away"), "停在别处却报贴底：\(scrollState())")
        // 键盘弹起时按钮抬过输入栏：量一次真实几何（ov 是叠层底边，ch 是输入栏高度）。
        field.tap()
        field.typeText(String(repeating: "scroll target sample: long message. ", count: 3))
        let raised = debugValues()
        guard raised["kb"] == 1 else {
            return XCTFail("打了字键盘没起来，量不了抬升：\(scrollState())")
        }
        guard let lifted = countedBottomButton(3, timeout: 5) else {
            return XCTFail("键盘起来后按钮没了：\(buttonLabels())")
        }
        let composerTop = (raised["ov"] ?? 0) - (raised["ch"] ?? 0)
        XCTAssertLessThanOrEqual(lifted.frame.maxY, composerTop + 1,
                                 "按钮压进输入栏：maxY=\(lifted.frame.maxY) 输入栏顶=\(composerTop) \(scrollState())")
        let send = app.buttons["composerSend"]
        XCTAssertTrue(send.waitForExistence(timeout: 5), "打了字没有发送键")
        send.tap()
        XCTAssertTrue(waitNoElement("composerSend", timeout: 3), "发送键没消失，消息没发出去：\(scrollState())")
        XCTAssertTrue(noBottomButton(timeout: 5), "自己发消息以后没回到底部：\(scrollState())")
        // 发完以后又贴着底：再翻进历史，按钮还得回来。先等键盘和贴底动画停住。
        Thread.sleep(forTimeInterval: 1)
        XCTAssertTrue(scrollUpInRoom(times: 3), "发送后拖不上去：kb=\(app.keyboards.count) \(scrollState())")
        guard let again = anyBottomButton(timeout: 10) else {
            return XCTFail("发完再翻走没有回到底部按钮：\(buttonLabels()) \(scrollState())\n\(app.debugDescription)")
        }
        again.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        // 点过的那个句柄会一直报存在（XCUIElement 缓存），消失断言用现起的查询。
        XCTAssertTrue(noBottomButton(timeout: 5), "点过以后按钮还在：\(scrollState())")
        let state = scrollState()
        XCTAssertTrue(state.contains("pin=true") && isAtBottom(state), "点了没有回到底部：\(state)")
        XCTAssertTrue(state.contains("vp=bottom"), "点了还是停在别处：\(state)")
        shot("33-scroll-button-bottom")
    }

    /// #1092：惯性还在滑的时候点“回到底部”，要当场打断惯性、落到最新一条。
    /// 慢拖带 `thenHoldForDuration` 松手时没有速度，这里用不按住的快拖制造减速，趁 `decel=1` 的时候点。
    func testScrollToBottomInterruptsMomentum() throws {
        try launch(scheme: "light")
        guard enterFakeRoom() != nil else {
            return XCTFail("没进假房间")
        }
        // 进房落点的滚动放完再拖，别让两次减速叠在一起。
        Thread.sleep(forTimeInterval: 1)
        let list = app.collectionViews.firstMatch
        // 关掉“每个事件前等 App 空闲”的等待：惯性滑着时 App 一直不空闲，不关掉的话点按钮要等惯性停下才发出去。
        let swap = swapOutWaitForIdle()
        defer { if let swap { method_exchangeImplementations(swap.0, swap.1) } }
        XCTAssertNotNil(swap, "XCUITest 没有 `waitForQuiescenceIncludingAnimationsIdle:`，关不掉空闲等待")
        let tapped = tapBackToBottomDuringMomentum(list)
        XCTAssertNotNil(tapped, "没能趁惯性还在的时候点回到底部：\(scrollState())")
        let light = waitPinnedAtBottom(timeout: 5)
        attach("after tap \(light)")
        XCTAssertTrue(light.contains("pin=true") && isAtBottom(light), "惯性中点回到底部没落到最底：\(light)")
        shot("1092-momentum-bottom-light")

        // 切深色要重启进程：截图用的颜色是启动时定住的，模拟器运行时切系统外观不一定生效。
        app.terminate()
        try launch(scheme: "dark")
        guard enterFakeRoom() != nil else {
            return XCTFail("深色下没进假房间")
        }
        Thread.sleep(forTimeInterval: 1)
        let darkList = app.collectionViews.firstMatch
        let tappedDark = tapBackToBottomDuringMomentum(darkList)
        XCTAssertNotNil(tappedDark, "深色下没能趁惯性点回到底部：\(scrollState())")
        let dark = waitPinnedAtBottom(timeout: 5)
        XCTAssertTrue(dark.contains("pin=true") && isAtBottom(dark), "深色下惯性中点回到底部没落到最底：\(dark)")
        shot("1092-momentum-bottom-dark")
    }

    /// XCUITest 在每个合成事件前等 App 空闲的那个私有方法：换成空的实现，返回原来的两个 `Method` 给调用方换回去。
    /// Xcode 26 给这个方法加了 `isPreEvent:`，按带不带它依次找。
    private func swapOutWaitForIdle() -> (Method, Method)? {
        guard let processClass = objc_getClass("XCUIApplicationProcess") as? AnyClass else { return nil }
        let candidates: [(Selector, Selector)] = [
            (Selector(("waitForQuiescenceIncludingAnimationsIdle:isPreEvent:")),
             #selector(noWaitForIdle(_:isPreEvent:))),
            (Selector(("waitForQuiescenceIncludingAnimationsIdle:")),
             #selector(noWaitForIdle(_:)))
        ]
        for (originalSelector, replacementSelector) in candidates {
            guard let original = class_getInstanceMethod(processClass, originalSelector),
                  let replacement = class_getInstanceMethod(Self.self, replacementSelector)
            else { continue }
            method_exchangeImplementations(original, replacement)
            return (original, replacement)
        }
        return nil
    }

    @objc private func noWaitForIdle(_ idle: Bool, isPreEvent: Bool) {}
    @objc private func noWaitForIdle(_ idle: Bool) {}

    /// 快拖一下制造惯性，等 `decel=1` 时点“回到底部”，返回点完那一刻的自述。没等到返回 nil。
    private func tapBackToBottomDuringMomentum(_ list: XCUIElement) -> String? {
        for _ in 0..<4 {
            // 快拖不按住：松手时手指还在动，列表接着惯性滑。速度给得越大，松手后的惯性越长。
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            let end = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85))
            start.press(forDuration: 0.02, thenDragTo: end,
                        withVelocity: XCUIGestureVelocity(20000), thenHoldForDuration: 0)
            guard let button = anyBottomButton(timeout: 2) else { continue }
            // `roomScroll` 每 0.25 秒才重写一次：松手那一刻读到的可能还是拖动中的 decel=0，等它翻过来。
            var state = ""
            var decelerating = false
            let spin = Date().addingTimeInterval(1)
            while Date() < spin, !decelerating {
                state = scrollState()
                decelerating = state.contains("decel=1")
                if !decelerating { usleep(50_000) }
            }
            guard decelerating else { continue }
            attach("tap during momentum \(state)")
            button.tap()
            return state
        }
        return nil
    }

    /// 等这一下“回到底部”滚完并贴底，返回当时的自述。
    private func waitPinnedAtBottom(timeout: TimeInterval) -> String {
        let deadline = Date().addingTimeInterval(timeout)
        var state = scrollState()
        while Date() < deadline, !(state.contains("pin=true") && isAtBottom(state)) {
            usleep(50_000)
            state = scrollState()
        }
        return state
    }

    /// 未读上百：名字报真实条数，角标只画 99+（ADR-0071）。
    func testBottomBadgeShows99Plus() throws {
        try launch(environment: ["WONTON_UI_FAKE_ARRIVALS": "100"])
        guard enterFakeRoom(stayAway: true) != nil else {
            return XCTFail("没进假房间")
        }
        guard anyBottomButton(timeout: 10) != nil || scrollUpInRoom() else {
            return XCTFail("翻走以后没有回到底部按钮：\(buttonLabels()) \(scrollState())")
        }
        guard let button = anyBottomButton(timeout: 10) else {
            return XCTFail("翻走以后没有回到底部按钮：\(buttonLabels()) \(scrollState())")
        }
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, !button.label.contains("100") { usleep(300_000) }
        XCTAssertTrue(button.label.contains("100"), "名字没报真实条数：\(button.label)")
        XCTAssertTrue(button.staticTexts["99+"].firstMatch.waitForExistence(timeout: 5), "角标没有写成 99+")
        shot("34-scroll-button-99plus")
    }

    /// 进房间的贴底落点（ADR-0071）：假数据让落点就是最新一条（`WONTON_UI_FAKE_ANCHOR=last`），
    /// `placeAnchor` 走贴底那一支的 0.3s `scrollToBottom`。进房不点“向下”，直接看动画走完是不是贴在底边；
    /// 把它换回 `pin.pin()` 的瞬时跳会看不出动画半路，这条量的是落点本身。
    func testRoomEntryLandsAtBottom() throws {
        try launch(environment: ["WONTON_UI_FAKE_ANCHOR": "last"])
        let row = app.buttons["chatRoom.!ui-fake:home.example"]
        guard row.waitForExistence(timeout: 20), row.isHittable else {
            return XCTFail("没找到假房间：\(buttonLabels())")
        }
        row.tap()
        guard waitInRoom(timeout: 10) else {
            return XCTFail("没进假房间：\(buttonLabels())")
        }
        let deadline = Date().addingTimeInterval(5)
        var state = scrollState()
        while Date() < deadline, !(state.contains("pin=true") && isAtBottom(state)) {
            usleep(300_000)
            state = scrollState()
        }
        XCTAssertTrue(state.contains("pin=true"), "进房没贴底：\(state)")
        XCTAssertTrue(isAtBottom(state), "进房没停在底部：\(state)")
        // `anim=` 是平滑滚的次数：换回 `pin.pin()` 的瞬时跳这里就是 0。
        XCTAssertGreaterThanOrEqual(debugValues()["anim"] ?? 0, 1, "进房落点没走平滑滚动：\(state)")
    }
}

extension ChatPanelUITests {

    // MARK: - 摆台

    /// 整套前置。要真后端的用例传 `needsStack: true`：没有 `PAIRING_CODE` 就跳过，有就配上真开发栈。
    /// 其余用例一律用假数据（`WONTON_UI_FAKE`），夜跑带了配对码也不连后端：房间里的用例不该依赖开发栈装没装 Bot。
    private func launch(arguments: [String] = [], needsStack: Bool = false,
                        environment: [String: String] = [:], scheme: String? = nil) throws {
        app = XCUIApplication()
        app.launchArguments += arguments
        if needsStack {
            try UITestBackend.require()
            try UITestBackend.passPairingCode(to: app)
        } else {
            // 假数据界面是英文；每次一个干净的数据目录，不恢复上一次的场景状态（房间记着上次看到哪一条）。
            app.launchArguments += [
                "-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"
            ]
            app.launchEnvironment["WONTON_UI_FAKE"] = "1"
            app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
            // 交接走没走完也看测试探针，否则 `settleKeyHandoff` 要白等满超时。
            UITestBackend.enableStateProbe(to: app)
        }
        app.launchEnvironment.merge(environment) { _, new in new }
        // 这代模拟器运行时切系统外观不一定生效，截图时钉住 `WONTON_SHOT_SCHEME`（WontonTheme 读它）。
        if let scheme {
            app.launchEnvironment["WONTON_SHOT_SCHEME"] = scheme
        }

        UITestBackend.launch(app)
        if needsStack {
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
        // 面板页面存在账号数据里，同一账号前一用例加的页会留到下一用例；假数据每次启动都是初态。
        if needsStack { clearPanelPages() }
    }

    /// 面板页面存在账号数据里，同一账号前一用例加的页会留到下一用例。用例开头都要空面板，先删干净。
    private func clearPanelPages() {
        let panel = app.otherElements["chatPanel"]
        openPanel()
        let deadline = Date().addingTimeInterval(20)
        while panel.value as? String == "full", Date() < deadline {
            app.buttons["pageMore"].firstMatch.tap()
            guard let edit = waitAnyButton(["Edit Panel", "编辑面板"], timeout: 5) else { break }
            edit.tap()
            let removes = app.buttons.matching(NSPredicate(format: "label IN %@", ["Remove", "移除"]))
            while removes.firstMatch.exists, removes.firstMatch.isHittable {
                removes.firstMatch.tap()
            }
            tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        }
        if capsule.waitForExistence(timeout: 1) {
            capsule.tap()
        } else {
            // 空面板只有第 1 级，没有胶囊：再点顶栏按钮收起。
            app.buttons.matching(identifier: "panelOpen").firstMatch.tap()
        }
        XCTAssertTrue(waitGone(panel), "清完面板后收不起来")
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

    /// 空面板最多到第 1 级（ADR-0052）：房间的面板没有页面时先加一页，才推得到整页。
    /// 加开发栈自带的“模型设置”（Models）：装的应用多了，编辑页里的网址输入框排在屏幕外。
    private func addPageIfEmpty() {
        let add = app.buttons["panelAdd"]
        guard add.waitForExistence(timeout: 2) else { return }
        add.tap()
        let models = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Models")).firstMatch
        XCTAssertTrue(models.waitForExistence(timeout: 10), "编辑面板里没有 Models 这一页")
        models.tap()
        tapAnyButton(["Done", "完成", "完了"], timeout: 5, required: true)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "面板网页没加载出来")
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

    private func openPanel() {
        let button = app.buttons.matching(identifier: "panelOpen").allElementsBoundByIndex
            .first { $0.exists && $0.isHittable }
        if let button { return button.tap() }
        // 房间里的入口在 Bot 资料页（ADR-0052）：点顶栏头像进资料页，再点“打开面板”。
        let avatar = app.buttons["roomTopBarAvatar"]
        guard avatar.exists, avatar.isHittable else {
            return XCTFail("没有可达的面板入口按钮\n\(app.debugDescription)")
        }
        avatar.tap()
        let entry = app.buttons["botProfilePanelOpen"]
        XCTAssertTrue(entry.waitForExistence(timeout: 5), "Bot 资料页没有打开面板的按钮\n\(app.debugDescription)")
        // 资料页上面是头像、状态、网页入口和收起的详情，按钮在下面。
        for _ in 0..<4 where !entry.isHittable {
            app.swipeUp()
        }
        entry.tap()
    }

    /// 进 Bot 的房间，返回输入框。新配对的设备走完引导直接落在主 Bot 的房间里，这时不用再从列表点进去。
    private func enterBotRoom() -> XCUIElement? {
        UITestBackend.settleKeyHandoff(app)
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
            guard let button = anyBottomButton(timeout: tapped ? 2 : 10) else { break }
            tapped = true
            button.tap()
            _ = waitGone(button, timeout: 5)
        }
    }

    /// 进假房间（核心 `ui_fake.rs` 的 Echo）；`stayAway` 为真时停在落点上、不回到底部。
    private func enterFakeRoom(stayAway: Bool = false) -> XCUIElement? {
        let row = app.buttons["chatRoom.!ui-fake:home.example"]
        guard row.waitForExistence(timeout: 20), row.isHittable else { return nil }
        row.tap()
        guard waitInRoom(timeout: 10) else { return nil }
        if !stayAway { goToBottom() }
        return composer(timeout: 3)
    }

    /// 房间顶栏的头像只在房间里出现，和收发框一起齐了才算进去。
    private func waitInRoom(timeout: TimeInterval) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if app.buttons["roomTopBarAvatar"].exists, composer(timeout: 0.2) != nil { return true }
            usleep(300_000)
        }
        return false
    }

    /// 把列表拖进历史，直到回到底部按钮真的出现：拖完贴底会自己吸回去，只看「刚才离开过」会被吸回的瞬间骗过。
    /// 贴底时历史在上方，手指往下拖才进历史；再补一下 `swipeDown` 走惯性滚。
    @discardableResult
    private func scrollUpInRoom(times: Int = 3) -> Bool {
        let list = app.collectionViews.firstMatch
        for _ in 0..<2 where app.keyboards.firstMatch.exists {
            list.swipeDown(velocity: .slow)
            usleep(600_000)
        }
        for _ in 0..<times {
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
            start.press(forDuration: 0.05,
                        thenDragTo: list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85)),
                        withVelocity: .slow, thenHoldForDuration: 0.8)
            usleep(600_000)
            if anyBottomButton(timeout: 0) != nil { return true }
            list.swipeDown(velocity: .slow)
            usleep(600_000)
            if anyBottomButton(timeout: 0) != nil { return true }
        }
        return anyBottomButton(timeout: 1) != nil
    }

    /// 现在没有回到底部按钮（现起查询，不吃旧句柄的缓存）。
    private func noBottomButton(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if anyBottomButton(timeout: 0) == nil { return true }
            if Date() >= deadline { return false }
            _ = app.wait(for: .runningForeground, timeout: 0.3)
        }
    }

    /// 按标识现查的元素不在了。
    private func waitNoElement(_ identifier: String, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if !app.buttons[identifier].exists { return true }
            if Date() >= deadline { return false }
            _ = app.wait(for: .runningForeground, timeout: 0.3)
        }
    }

    /// `roomScroll` 里 `k=v` 的数值，`kb` 之类的布尔按 0/1 给。
    private func debugValues() -> [String: Double] {
        var values: [String: Double] = [:]
        for part in scrollState().split(separator: " ") {
            let pair = part.split(separator: "=", maxSplits: 1)
            guard pair.count == 2 else { continue }
            values[String(pair[0])] = Double(pair[1]) ?? (pair[1] == "true" ? 1 : 0)
        }
        return values
    }

    private func scrollOffsetY() -> Double {
        debugValues()["y"] ?? -1
    }

    /// 带数字的完整标签（四种语言的本地化串），断言用整串相同、不用子串。
    private static let bottomCountFormats = [
        "Back to bottom, %1$@ new messages",
        "回到底部，还有 %1$@ 条新消息",
        "回到底部，還有 %1$@ 條新訊息",
        "一番下へ戻る、新着 %1$@ 件"
    ]

    /// 失败时把界面上的按钮标签都列出来。
    private func buttonLabels() -> [String] {
        (0..<app.buttons.count).map { app.buttons.element(boundBy: $0).label }
    }

    /// 带不带数字都是同一个按钮：数字版的标签是「回到底部，3」。
    private func anyBottomButton(timeout: TimeInterval) -> XCUIElement? {
        let labels = ["Back to bottom", "回到底部", "一番下へ戻る"]
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

    /// 数字等于 `count` 的那版：用能自己重算的查询，取到的是按当前标签绑定的元素。
    private func countedBottomButton(_ count: Int, timeout: TimeInterval) -> XCUIElement? {
        let matches = Self.bottomCountFormats.map { format in
            NSPredicate(format: "label == %@", String(format: format, "\(count)"))
        }
        let query = app.buttons.matching(NSCompoundPredicate(orPredicateWithSubpredicates: matches))
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if query.firstMatch.exists { return query.firstMatch }
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
               listBottom: list.frame.maxY, fieldTop: field.frame.minY, back: anyBottomButton(timeout: 0) != nil)
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

    /// 胶囊的退出按钮。按标识找：聊天页的提示条上也有叫“关闭”的按钮，按名字找会把它当成胶囊还在。
    private var capsule: XCUIElement {
        app.buttons.matching(identifier: "pageClose").firstMatch
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
