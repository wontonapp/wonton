import XCTest

/// 整页插件页的出口（ADR-0095，盲测 P0）：从侧边栏“我的应用”打开应用，胶囊一直在、切后台回来还在、点了回聊天；
/// 左边缘右滑回聊天；页面中间的横滑归网页，不返回。
///
/// 要连装好了这些应用的开发栈：外部传 `PAIRING_CODE`（见 ChatPanelUITests）。没装的应用那一条直接失败。
final class PluginPageUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 配对、引导、等网页、切后台再回来、开两次：一条走下来超过默认的 90 秒。
        executionTimeAllowance = 300
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
        try walk("Immich", arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
    }

    func testNavidromeLargestTextDark() throws {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        try walk("Navidrome", arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
    }

    private func walk(_ name: String, arguments: [String] = []) throws {
        try launch(arguments: arguments)
        open(name)
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 60), "\(name) 的网页没出来")
        // 胶囊以前 3 秒没碰就藏起来：等过这段时间还得在、点得到。
        Thread.sleep(forTimeInterval: 5)
        assertCapsule("\(name) 打开后")
        shot("\(name)-01-open")

        // 盲测第二轮：页面往下滚以后原来的顶栏收起，就再没有出口。滚过之后胶囊还得在。
        drag(from: CGVector(dx: 0.5, dy: 0.75), to: CGVector(dx: 0.5, dy: 0.25))
        assertCapsule("\(name) 页面往下滚后")

        // 页面中间的横滑是网页自己的，不能把页面关掉。
        drag(from: CGVector(dx: 0.3, dy: 0.6), to: CGVector(dx: 0.9, dy: 0.6))
        assertCapsule("\(name) 页面中间横滑后")

        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来")
        assertCapsule("\(name) 切后台再回来")
        shot("\(name)-02-after-home")
        capsule.tap()
        XCTAssertTrue(waitGone(capsule), "\(name) 点胶囊关闭后页面还在")
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "\(name) 关闭后没回到聊天")
        shot("\(name)-03-closed")

        open(name)
        XCTAssertTrue(capsule.waitForExistence(timeout: 30), "\(name) 第二次打开没有胶囊")
        drag(from: CGVector(dx: 0.005, dy: 0.5), to: CGVector(dx: 0.85, dy: 0.5))
        XCTAssertTrue(waitGone(capsule), "\(name) 左边缘右滑没返回")
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), "\(name) 左边缘右滑后没回到聊天")
        shot("\(name)-04-edge-back")
    }

    /// 侧边栏“我的应用”里有就从那里开；新配上的设备“我的应用”是空的（配对前装的不自动加），从“全部应用”网格开。
    private func open(_ name: String) {
        if let row = UITestSidebar.row(app, labels: [], containing: name) {
            row.tap()
            return
        }
        let allApps = UITestSidebar.allApps(app)
        XCTAssertNotNil(allApps, "侧边栏里既没有 \(name) 也没有“全部应用”\n\(app.debugDescription)")
        allApps?.tap()
        let icon = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS[c] %@", name)).firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 20), "“全部应用”里没有 \(name)（开发栈没装？）\n\(app.debugDescription)")
        icon.tap()
    }

    private func assertCapsule(_ when: String) {
        XCTAssertTrue(capsule.waitForExistence(timeout: 5), "\(when)没有胶囊\n\(app.debugDescription)")
        XCTAssertTrue(capsule.isHittable && app.frame.contains(capsule.frame), "\(when)胶囊被挤出屏幕或点不到")
    }

    private func drag(from start: CGVector, to end: CGVector) {
        app.coordinate(withNormalizedOffset: start)
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: end),
                   withVelocity: .fast, thenHoldForDuration: 0)
    }

    private var capsule: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label IN %@", ["Close", "关闭", "關閉", "閉じる"])).firstMatch
    }

    /// 照 ChatPanelUITests 的启动：等配对完成，走完引导，到聊天列表。等待都短于单条测试的 90 秒上限，
    /// 卡住时能把当时的界面树打出来。
    private func launch(arguments: [String]) throws {
        app = XCUIApplication()
        app.launchArguments += arguments
        if let code = try UITestBackend.codes().first {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
        app.launch()
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
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 5), "聊天页上没有侧边栏按钮\n\(app.debugDescription)")
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
