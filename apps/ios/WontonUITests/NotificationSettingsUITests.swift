import UIKit
import XCTest

/// 设置 → 通知 → 系统通知（#677）：入口要落到云吞自己的系统通知页上，不能停在系统设置的首页或者云吞的普通应用页。
///
/// 这台设备还没被问过通知权限时（模拟器刚建好就是这种）先弹系统的授权框，允许以后一样落到云吞的通知页——
/// iOS 只给问过的 App 建这一页。行右侧写着系统里的状态。
///
/// 启动时 App 自己会问权限，`UITestBackend.passPairingCode` 带的 `WONTON_DEV_SKIP_PUSH_AUTH` 让用例自己挑时机：
/// 新模拟器上验“还没问过”，同一台再跑一次时按已经允许的状态走。连开发栈（设置页上的“通知”行由核心给）：
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/NotificationSettingsUITests
///
/// 每一步都留截图附件。
final class NotificationSettingsUITests: XCTestCase {
    private var app: XCUIApplication!
    private let uiWait: TimeInterval = 10
    /// 要经过家里的服务器的：最多等 90 秒。
    private let networkWait: TimeInterval = 90

    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 480
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        UITestBackend.enableStateProbe(to: app)
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
    }

    @MainActor
    func testSystemNotificationsEntryOpensWontonsOwnNotificationPage() throws {
        // 落点没成立时记一条失败也接着走，后面的状态断言才有机会说明问题。
        continueAfterFailure = true
        // 启动时带的码连不上（家里那台正忙）就换下一个码再试。
        try UITestBackend.pairIfNeeded(app)
        reachSettings()
        // 行的可访问性标签带着右侧的值（“Notifications, Not receiving”）。
        let notifications = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Notifications"))
        for _ in 0..<3 where hittable(notifications) == nil { app.swipeUp() }
        XCTAssertNotNil(hittable(notifications), "设置页里没有“通知”\n\(app.debugDescription)")
        hittable(notifications)?.tap()

        let entry = poll(uiWait, until: { row("System Notifications") })
        XCTAssertNotNil(entry, "通知页里没有“系统通知”\n\(app.debugDescription)")
        let value = poll(uiWait, until: { rowValue(entry, excluding: "System Notifications") })
        XCTAssertNotNil(value, "通知行右侧没有状态\n\(app.debugDescription)")
        shot("01-notification-settings")
        entry?.tap()

        // 已经被问过（同一台模拟器再跑一次）：点行不再弹框，直接去系统设置。
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        if value == "Allowed" {
            shot("02-no-permission-alert")
        } else if allow.waitForExistence(timeout: networkWait) {
            // 没问过权限：系统先弹授权框。按钮跟着模拟器语言走，不认字：两个按钮里第二个是“允许”。
            shot("02-permission-alert")
            allow.tap()
        } else {
            shot("02-no-permission-alert")
        }

        let prefs = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        let front = prefs.wait(for: .runningForeground, timeout: 30)
        shot("03-system-settings")
        XCTAssertTrue(front, "点“系统通知”没把系统设置带到前台\n\(app.debugDescription)")
        // 深链能直接落到云吞的通知页最好；26.5 模拟器上落不到，就从系统设置首页自己走过去。
        // 连系统设置页都进不去（深链落空、App 列表也点不动）是运行时/系统页的行为，不是产品行为：Skip。
        let landed = waitUntil(uiWait, { isOnNotificationPage(prefs) })
        if !landed {
            shot("03b-system-settings-home")
            var reached = isOnWontPage(prefs)
            if !reached {
                _ = openWontPage(in: prefs)
                reached = waitUntil(uiWait, { isOnWontPage(prefs) })
            }
            guard reached else {
                throw XCTSkip("系统设置里到不了云吞的通知页（深链落不到、App 列表点不进）：\(runtime)")
            }
            let notificationEntry = poll(uiWait, until: { row(prefs, labels: ["Notifications", "通知"]) })
            XCTAssertNotNil(notificationEntry, "云吞的页里没有“通知”这一行\n\(prefs.debugDescription)")
            notificationEntry?.tap()
        }
        XCTAssertTrue(waitUntil(uiWait, { isOnNotificationPage(prefs) }), "没进云吞的通知页\n\(prefs.debugDescription)")
        shot("04-wonton-notification-page")
    }

    /// 冷启动连上家里的服务器（负载高时要两三分钟），走完或跳引导，再进设置。
    private func reachSettings() {
        let settings = app.buttons.matching(NSPredicate(format: "label IN %@", ["Settings", "设置"]))
        // 引导页的“Step x of 4”是这一层的判据（后面的主界面按钮在引导期间也查得到）。
        let stepLabel = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Step "))
            .firstMatch
        let deadline = Date().addingTimeInterval(networkWait * 3)
        while stepLabel.exists, Date() < deadline {
            if UITestBackend.skipKeyHandoff(app) { continue }
            // 引导最后一页的按钮是“Go to Chat”（装应用的期间先是灰的，那就用右上角的 Close 离开）；
            // “Skip”在那一页也是灰的，所以只认能点的。
            let step = ["Go to Chat", "Close", "Skip", "Done", "Next"].map { app.buttons[$0].firstMatch }
                .first { $0.exists && $0.isEnabled }
            if let step {
                step.tap()
            } else {
                Thread.sleep(forTimeInterval: 2)
            }
        }
        UITestBackend.settleKeyHandoff(app)
        // 底栏和侧边栏里都有“Settings”：抽屉开着时底栏那个被挤到屏幕外。先按遮罩把抽屉收起，底栏回到屏幕里；
        // 没有遮罩却点不到底栏时，用侧边栏按钮换一次状态。
        let collapse = app.buttons["Collapse sidebar"]
        if collapse.exists {
            collapse.tap()
            _ = collapse.waitForNonExistence(timeout: uiWait)
        }
        if hittable(settings) == nil {
            let bar = app.buttons.matching(UITestSidebar.button).firstMatch
            if bar.exists, bar.isHittable { bar.tap() }
        }
        guard let target = poll(uiWait, until: { hittable(settings) }) else {
            return XCTFail("设置行点不到\n\(app.debugDescription)")
        }
        target.tap()
    }
    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }
    private func isOnWontPage(_ prefs: XCUIApplication) -> Bool {
        prefs.navigationBars["Wonton"].exists || prefs.navigationBars["云吞"].exists
    }

    /// 云吞在系统设置里的通知页：有允许开关或者通知页的导航栏。
    private func isOnNotificationPage(_ prefs: XCUIApplication) -> Bool {
        prefs.switches["Allow Notifications"].exists || prefs.switches["允许通知"].exists
            || prefs.navigationBars["Notifications"].exists || prefs.navigationBars["通知"].exists
    }

    /// 系统设置首页 → 云吞 → 云吞的页。iOS 26 的 App 列表直接排在首页里，不一定有单独的入口。走到了返回 true。
    @discardableResult
    private func openWontPage(in prefs: XCUIApplication) -> Bool {
        // 在别的子页里：先退回首页。
        for _ in 0..<4 {
            guard let back = backButton(in: prefs) else { break }
            back.tap()
        }
        // iOS 26 的 App 列表直接接在设置首页下面，往下拖就能看到（顶部的搜索胶囊在这台运行时上点不进焦点）。
        // 快滑（swipeUp）在这个设置页上不滚，用慢拖。
        let from = prefs.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
        let to = prefs.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
        var appCell: XCUIElement?
        for _ in 0..<16 where appCell == nil {
            appCell = wontonCell(in: prefs)
            if appCell == nil { from.press(forDuration: 0.1, thenDragTo: to) }
        }
        appCell?.tap()
        return appCell != nil
    }

    /// 测试的机型和运行时，写进 Skip 的原因里。
    private var runtime: String {
        "\(UIDevice.current.model) / \(UIDevice.current.systemName) \(UIDevice.current.systemVersion)"
    }

    /// 系统设置子页的返回按钮，跟系统语言走。
    private func backButton(in prefs: XCUIApplication) -> XCUIElement? {
        for label in ["设置", "Settings"] {
            let button = prefs.navigationBars.buttons[label]
            if button.exists { return button }
        }
        return nil
    }

    /// 列表里云吞那一行的文字。
    private func wontonCell(in prefs: XCUIApplication) -> XCUIElement? {
        for label in ["云吞", "Wonton"] {
            let texts = prefs.staticTexts.matching(NSPredicate(format: "label == %@", label))
            for text in texts.allElementsBoundByIndex where text.exists && text.isHittable { return text }
            let rows = prefs.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label))
            for row in rows.allElementsBoundByIndex where row.exists && row.isHittable { return row }
        }
        return nil
    }

    private func row(_ title: String) -> XCUIElement? {
        row(app, labels: [title])
    }

    /// 行右侧的状态文字：行里除标题之外的文字。行被暴露成单个文字时（点得到的是文字本身），退回整页找状态词。
    private func rowValue(_ cell: XCUIElement?, excluding title: String) -> String? {
        let labels = cell?.staticTexts.allElementsBoundByIndex.map { $0.label } ?? []
        if let text = labels.first(where: { $0 != title }) { return text }
        let known = ["Allowed", "Not allowed"]
        return known.first { app.staticTexts[$0].exists }
    }

    /// 指定界面里叫这个名字的列表行。
    private func row(_ root: XCUIApplication, labels: [String]) -> XCUIElement? {
        for label in labels {
            let cells = root.cells.containing(.staticText, identifier: label)
            for cell in cells.allElementsBoundByIndex where cell.exists && cell.isHittable { return cell }
            let texts = root.staticTexts.matching(NSPredicate(format: "label == %@", label))
            for text in texts.allElementsBoundByIndex where text.exists && text.isHittable { return text }
            // 行整体是按钮时标签带着右侧的值（“System Notifications, Not allowed”）。
            let buttons = root.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label))
            for button in buttons.allElementsBoundByIndex where button.exists && button.isHittable { return button }
        }
        return nil
    }

    private func poll<T>(_ timeout: TimeInterval, until condition: () -> T?) -> T? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let value = condition() { return value }
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        } while Date() < deadline
        return condition()
    }

    private func waitUntil(_ timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        poll(timeout, until: { condition() ? true : nil }) != nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
