import XCTest

/// 走查（ADR-0033 第 1 件事）：Navidrome 放歌 → 切后台 60 秒（这期间走查人在外面重启栈的核心容器断一次隧道）
/// → 回前台歌还在放，或页面还活着、点一下播放能续上；日志里找 "walkthrough:" 行配合外面的动作。
///
/// 要连装好 Navidrome 的开发栈（同 PluginPageUITests）；没有配对码整条用例跳过。跑一次约 3 分钟，
/// 供走查取证，也进夜跑（有栈时）。
final class NavidromeMediaWalkthroughUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try XCTSkipIf(try UITestBackend.codes().isEmpty, "需要开发栈：没有 PAIRING_CODE")
    }

    func testPlaybackSurvivesBackgroundAndTunnelRestart() throws {
        executionTimeAllowance = 300
        app = XCUIApplication()
        try UITestBackend.passPairingCode(to: app)
        app.launch()
        XCTAssertTrue(waitPaired(timeout: 40), "配对没完成\n\(app.debugDescription)")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 15), "没到聊天页\n\(app.debugDescription)")
        open("Navidrome")
        startPluginIfNeeded()
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 90), "Navidrome 的网页没出来")
        attach("navidrome-01-open")

        try startPlayback()
        XCTAssertTrue(waitPlaying(timeout: 15), "点了播放但没看到暂停钮（没放起来）\n\(app.debugDescription)")
        attach("navidrome-02-playing")

        // 切后台 60 秒；走查人看到 "walkthrough: backgrounded" 就重启栈的核心容器，恢复后测试再回前台。
        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 10)
        NSLog("walkthrough: backgrounded; 现在断隧道（重启核心容器），60 秒后恢复")
        Thread.sleep(forTimeInterval: 60)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回前台")
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 30), "回前台网页没了")
        attach("navidrome-03-foreground")

        // 隧道断过又恢复：歌还挂着暂停钮最好；断了就再点一次播放，10 秒内放起来也算续上。
        if !isPlaying {
            NSLog("walkthrough: 回前台歌停了，点播放续上")
            tapPlayControl()
            XCTAssertTrue(waitPlaying(timeout: 15), "隧道恢复后没能续上播放\n\(app.debugDescription)")
        }
        XCTAssertTrue(isPlaying, "回前台不在放\n\(app.debugDescription)")
        attach("navidrome-04-resumed")
        NSLog("walkthrough: done; 歌在断隧道恢复后还在放")
    }

    // MARK: - 播放控制

    /// 网页里播放/暂停一类的钮：Navidrome 播放条上的播放钮和专辑卡上的播放钮都带 Play/Pause 标签。
    private var playControls: XCUIElementQuery {
        app.webViews.buttons.matching(NSPredicate(
            format: "label IN[c] %@", ["Play", "Pause", "Play or pause", "play", "pause"]
        ))
    }

    private var isPlaying: Bool {
        // 放着的时候播放钮变成暂停。
        app.webViews.buttons.matching(NSPredicate(format: "label ==[c] %@", ["Pause", "pause"])).firstMatch.exists
    }

    private func waitPlaying(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if isPlaying { return true }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return isPlaying
    }

    private func tapPlayControl() {
        let play = app.webViews.buttons.matching(NSPredicate(format: "label ==[c] %@", ["Play", "play"])).firstMatch
        XCTAssertTrue(play.waitForExistence(timeout: 10), "网页里没有播放钮\n\(app.debugDescription)")
        play.tap()
    }

    /// 首页第一张专辑卡上的播放钮直接放；没有就进第一张专辑再点页头播放。
    private func startPlayback() throws {
        tapPlayControl()
    }

    /// 插件是懒启动：页面出「已停止 / Start」就点启动，等容器跑起来再等网页。
    private func startPluginIfNeeded() {
        let start = app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Start", "启动", "啟動", "起動"]
        )).firstMatch
        if start.waitForExistence(timeout: 5) { start.tap() }
    }

    // MARK: - 摆台（同 PluginMediaUITests）

    /// 侧边栏“我的应用”里有就从那里开；没有就从“全部应用”网格开。配对后插件列表要从栈上加载，给足 30 秒。
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

    private func attach(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private static let onboardingButtons = [
        "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成", "Next", "下一步", "Skip", "跳过"
    ]
}
