import XCTest

/// 网页要麦克风的壳询问（ADR-0033）：首次弹壳的确认，答了允许走系统权限框、答了拒绝不弹系统框；
/// 核心按插件记决定，再开（页面重载后再要一次）不再问。
///
/// 真插件页没有现成的麦克风入口，XCUITest 也执行不了网页脚本：App 里装了 `wonton-media-capture-test`
/// 参数时（DEBUG），插件页每次加载完自己在页面里要一次麦克风（结果在模拟器日志的 "Wonton media probe"）。
/// 要连装好 Navidrome 的开发栈（同 PluginPageUITests）；没有配对码整条用例跳过。
final class PluginMediaUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try XCTSkipIf(try UITestBackend.codes().isEmpty, "需要开发栈：没有 PAIRING_CODE")
    }

    /// 首次要麦克风弹壳的确认，答允许后走系统权限框；重载后再要，核心记住了，两个框都不再出。
    func testMicrophonePromptAllowsAndRemembers() throws {
        try run { prompt in
            prompt.allow.tap()
            // 壳放行了才申请系统运行时权限；模拟器上一回跑给过的话系统框不再出，只等 5 秒。
            let systemAllow = springboard.buttons.matching(
                NSPredicate(format: "label IN %@", Self.systemAllowLabels)
            ).firstMatch
            _ = systemAllow.waitForExistence(timeout: 5)
            if systemAllow.exists { systemAllow.tap() }
        }
    }

    /// 首次答不允许：不弹系统框；重载后再要，核心记住了，壳的确认不再出。
    func testMicrophonePromptDenyRemembers() throws {
        try run { prompt in
            prompt.deny.tap()
            XCTAssertFalse(springboard.alerts.firstMatch.waitForExistence(timeout: 5), "答了不允许还弹系统框")
        }
    }

    // MARK: - 摆台

    /// 首次要麦克风时壳的确认弹窗必须出；答允许后系统权限框出来（上一次跑给过权限的话它不再出，
    /// 等 5 秒就算了），答不允许不弹系统框；重载后再要，核心记住了，两个框都不再出。
    private func run(_ answer: (Prompt) throws -> Void) throws {
        executionTimeAllowance = 300
        app = XCUIApplication()
        app.launchArguments += ["wonton-media-capture-test"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
        XCTAssertTrue(waitPaired(timeout: 40), "配对没完成（配对码过期、用过或连不上开发栈）\n\(app.debugDescription)")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 15), "没到聊天页\n\(app.debugDescription)")
        open("Navidrome")
        startPluginIfNeeded()
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 90), "Navidrome 的网页没出来")

        let prompt = Prompt(app: app)
        XCTAssertTrue(prompt.title.waitForExistence(timeout: 20), "首次要麦克风没出壳的确认弹窗\n\(app.debugDescription)")
        try answer(prompt)
        XCTAssertTrue(waitGone(prompt.title), "答完弹窗还在\n\(app.debugDescription)")

        // 重载页面再要一次：决定已按插件记下，壳的确认和系统框都不再出（本地等待 ≤10 秒）。
        let more = app.buttons.matching(NSPredicate(format: "label IN %@", ["More", "更多", "その他"])).firstMatch
        XCTAssertTrue(more.waitForExistence(timeout: 10), "胶囊上没有菜单\n\(app.debugDescription)")
        more.tap()
        let reload = app.buttons.matching(NSPredicate(format: "label IN %@", ["Reload", "重新加载", "再読み込み"])).firstMatch
        XCTAssertTrue(reload.waitForExistence(timeout: 10), "菜单里没有重新加载\n\(app.debugDescription)")
        reload.tap()
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "重载后网页没出来")
        XCTAssertFalse(prompt.title.waitForExistence(timeout: 8), "重载后又出了壳的确认：决定没被记住")
        XCTAssertFalse(springboard.alerts.firstMatch.waitForExistence(timeout: 2), "重载后又出了系统权限框")
    }

    /// 插件是懒启动：页面出「已停止 / Start」就点启动，等容器跑起来再等网页。
    private func startPluginIfNeeded() {
        let start = app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Start", "启动", "啟動", "起動"]
        )).firstMatch
        if start.waitForExistence(timeout: 5) { start.tap() }
    }

    /// 壳的确认弹窗：标题带插件名和“麦克风”，允许和不允许两个按钮。
    private struct Prompt {
        let title: XCUIElement
        let allow: XCUIElement
        let deny: XCUIElement

        init(app: XCUIApplication) {
            title = app.staticTexts.matching(NSPredicate(
                format: "label CONTAINS 'use your microphone' OR label CONTAINS '使用麦克风'"
            )).firstMatch
            allow = app.buttons.matching(NSPredicate(format: "label IN %@", ["Allow", "允许", "允許", "許可"])).firstMatch
            deny = app.buttons.matching(NSPredicate(format: "label IN %@", ["Don't allow", "不允许", "不允許", "許可しない"])).firstMatch
        }
    }

    /// 系统的权限框在 SpringBoard 里，不在被测 App 的层级。
    private var springboard: XCUIApplication { XCUIApplication(bundleIdentifier: "com.apple.springboard") }

    /// 引导页之间有要装的 Agent（新栈首次配对装 Hermes），转场可能超过几秒：单轮等 6 秒、
    /// 连续三轮找不到按钮才收，别一轮没找到就退出（11-13 走查实测卡死在最后一页 Review）。
    private func passOnboarding() {
        var missed = 0
        for _ in 0..<24 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            guard let button = waitAnyButton(Self.onboardingButtons, timeout: 6) else {
                missed += 1
                if missed >= 3 { break }
                continue
            }
            missed = 0
            button.tap()
        }
    }

    private static let systemAllowLabels = ["OK", "Allow", "允许", "允許", "許可"]
    private static let onboardingButtons = [
        "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成", "Next", "下一步", "Skip", "跳过"
    ]

    /// 侧边栏“我的应用”里有就从那里开；没有就从“全部应用”网格开。配对后插件列表要从栈上加载，
    /// 等待给足 30 秒（“全部应用”一行在列表到达前不存在）。
    private func open(_ name: String) {
        if let row = UITestSidebar.row(app, labels: [], containing: name, timeout: 30) {
            row.tap()
            return
        }
        let allApps = UITestSidebar.allApps(app, timeout: 30)
        XCTAssertNotNil(allApps, "侧边栏里既没有 \(name) 也没有“全部应用”\n\(app.debugDescription)")
        allApps?.tap()
        let icon = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS[c] %@", name)).firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 20), "“全部应用”里没有 \(name)（开发栈没装？）\n\(app.debugDescription)")
        icon.tap()
    }

    /// 配对页的输入框不在、又已经出了页面（聊天列表或引导）才算配上（同 PluginPageUITests）。
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
}
