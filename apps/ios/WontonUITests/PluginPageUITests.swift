import XCTest

/// 整页插件页的出口（ADR-0095，盲测 P0）：从侧边栏“我的应用”打开应用，顶上没有胶囊，网页框从状态栏下沿开始；
/// 往下滚、切后台回来页面还在；左边缘右滑回聊天，两次都行；页面中间的横滑归网页，不返回。
///
/// 要连装好了这些应用的开发栈：外部传 `PAIRING_CODE`，没有就跳过。没装的应用那一条直接失败。
final class PluginPageUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 配对、引导、等网页、切后台再回来、开两次：一条走下来超过默认的 90 秒。
        executionTimeAllowance = 300
        try XCTSkipIf(try UITestBackend.codes().isEmpty, "需要开发栈：没有 PAIRING_CODE")
    }

    func testImmich() throws { try walk("Immich") }

    func testNavidrome() throws { try walk("Navidrome") }

    func testActual() throws { try walk("Actual") }

    /// 盲测第二轮在“模型设置”（magpie 的网页，英文名 Models）里也困住过。
    func testModelSettings() throws { try walk("Models") }

    /// 盲测里最大的辅助字号和深色下出口被挤掉或看不见过。
    func testImmichLargestTextDark() throws {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        try walk(
            "Immich",
            arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        )
    }

    func testNavidromeLargestTextDark() throws {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        try walk(
            "Navidrome",
            arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        )
    }

    /// 回前台后立刻打开插件页：首个非空内容 2 秒内出现（取证的修前红、修后绿）。10 轮“后台 30 秒 → 回前台 →
    /// 打开”，2 秒从 `open()` 返回（点按之后）算；看的是 WebView 的 DOM，skeleton 可能还盖着。
    ///
    /// 2 秒盯的是页面自己（和第 1 轮真红时 5.25 秒里那 2 秒空白同量级）；点按侧边栏、等抽屉展开这些是 XCTest 的
    /// 合成输入，模拟器上稳定要 3 秒上下，另用一条宽的总时限兜住整条路的回归。
    func testContentWithinTwoSecondsAfterResume() throws {
        // 夜跑的 -maximum-test-execution-time-allowance 是 600：留出余量。
        executionTimeAllowance = 580
        try launch(arguments: [])
        // 先开一次关掉：全新设备的插件列表和网页都是冷的，枚举前把“我的应用”和隧道预热好，
        // 免得第 1 轮把冷启动算进去。
        open(Self.resumeApp)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "预热时网页没出来")
        swipeBack()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "预热后回不到聊天")
        var slow: [String] = []
        for round in 1...10 {
            UITestLifecycle.backgroundAndReturn(app, linger: 0, background: 30)
            let started = Date()
            open(Self.resumeApp)
            let opened = Date().timeIntervalSince(started)
            let shown = waitForFirstContent(timeout: 2)
            let total = Date().timeIntervalSince(started)
            let elapsed = String(format: "%.2f", total)
            print("resume \(round): \(elapsed) s visible=\(shown) open=\(String(format: "%.2f", opened))")
            if !shown { slow.append("\(round):\(elapsed)") }
            XCTAssertLessThan(total, 6, "第 \(round) 轮整条路（含点按）超过 6 秒")
            shot("resume-\(round)")
            swipeBack()
            XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "第 \(round) 轮回不到聊天")
        }
        XCTAssertTrue(slow.isEmpty, "回前台 2 秒内没有内容的轮次（轮:秒）：\(slow.joined(separator: " "))")
    }

    private func walk(_ name: String, arguments: [String] = []) throws {
        try launch(arguments: arguments)
        open(name)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 60), "\(name) 的网页没出来")
        assertPage("\(name) 打开后")
        shot("\(name)-01-open")

        // 盲测第二轮：页面往下滚以后原来的顶栏收起，就再没有出口。滚过之后左边缘右滑还得管用（下面走）。
        drag(from: CGVector(dx: 0.5, dy: 0.75), to: CGVector(dx: 0.5, dy: 0.25))
        assertPage("\(name) 页面往下滚后")

        // 页面中间的横滑是网页自己的，不能把页面关掉。
        drag(from: CGVector(dx: 0.3, dy: 0.6), to: CGVector(dx: 0.9, dy: 0.6))
        assertPage("\(name) 页面中间横滑后")

        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来")
        assertPage("\(name) 切后台再回来")
        shot("\(name)-02-after-home")
        swipeBack()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "\(name) 左边缘右滑后没回到聊天")
        shot("\(name)-03-closed")

        open(name)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "\(name) 第二次打开网页没出来")
        swipeBack()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "\(name) 第二次左边缘右滑后没回到聊天")
        shot("\(name)-04-edge-back")
    }

    /// 回前台用例用的应用：栈上装的是 whoami 示例插件，页面极小，内容就是回显。
    private static let resumeApp = "Who am I"

    /// 网页里出现第一个非空内容：文字、图片或按钮。轮询间隔要远小于 2 秒。
    private func waitForFirstContent(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let web = app.webViews.firstMatch
            if web.exists, web.staticTexts.count > 0 || web.images.count > 0 || web.buttons.count > 0 {
                return true
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        return false
    }

    private func open(_ name: String) {
        guard let row = UITestSidebar.row(app, labels: [], containing: name) else {
            return XCTFail("新设备的“我的应用”里没有 \(name)\n\(app.debugDescription)")
        }
        shot("\(name)-my-apps")
        row.tap()
    }

    /// 页面还开着，顶上没有胶囊：网页框从状态栏下沿开始，页面自己固定在顶上的按钮不被压住。
    private func assertPage(_ when: String) {
        let web = app.webViews.firstMatch
        XCTAssertTrue(web.waitForExistence(timeout: 5), "\(when)页面不在了\n\(app.debugDescription)")
        XCTAssertFalse(capsule.exists, "\(when)整页上还有胶囊")
        XCTAssertFalse(app.buttons.matching(UITestSidebar.button).firstMatch.isHittable, "\(when)回到了聊天")
        XCTAssertTrue(web.frame.minY >= 40 && web.frame.minY <= 70, "\(when)网页框的顶边不在状态栏下沿：\(web.frame)")
    }

    /// 推入的转场走完系统才认返回手势。
    private func swipeBack() {
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
    }

    private func drag(from start: CGVector, to end: CGVector) {
        app.coordinate(withNormalizedOffset: start)
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: end),
                   withVelocity: .fast, thenHoldForDuration: 0)
    }

    private var capsule: XCUIElement {
        app.buttons.matching(identifier: "pageClose").firstMatch
    }

    /// 启动：等配对完成，走完引导，到聊天列表。等待都短于单条测试的 90 秒上限，
    /// 卡住时能把当时的界面树打出来。
    private func launch(arguments: [String]) throws {
        app = XCUIApplication()
        app.launchArguments += arguments
        if let code = try UITestBackend.codes().first {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
        UITestBackend.enableStateProbe(to: app)
        UITestBackend.launch(app)
        if app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] != nil {
            XCTAssertTrue(waitPaired(timeout: 40), "配对没完成（配对码过期、用过或连不上开发栈）\n\(app.debugDescription)")
        }
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            guard let button = waitAnyButton(Self.onboardingButtons, timeout: 3) else { break }
            button.tap()
        }
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 15), "没到聊天页\n\(app.debugDescription)")
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 30), "聊天页上没有侧边栏按钮\n\(app.debugDescription)")
    }

    private static let onboardingButtons = [
        "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成", "Next", "下一步", "Skip", "跳过"
    ]

    /// 配对页的输入框不在、又已经出了页面（聊天列表或引导）才算配上；出现拒绝提示就不用再等。
    private func waitPaired(timeout: TimeInterval) -> Bool {
        let anyElement = app.descendants(matching: .any)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if anyElement["pairing-refusal"].exists { return false }
            let rendered = app.scrollViews.firstMatch.exists || app.buttons["Skip"].exists || app.buttons["跳过"].exists
            if rendered && !anyElement["pairing-code"].exists { return true }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return false
    }

    /// 引导最后一页的“跳过”还在、只是禁用：只认能点的。
    private func waitAnyButton(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists, element.isEnabled, element.isHittable { return element }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    private func waitGone(_ element: XCUIElement, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !element.exists { return true }
            Thread.sleep(forTimeInterval: 0.3)
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
