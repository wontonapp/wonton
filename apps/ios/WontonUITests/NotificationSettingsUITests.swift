import XCTest

/// 设置 → 通知 → 系统通知（#677）：入口要落到云吞自己的系统通知页上，不能停在系统设置的首页或者云吞的普通应用页。
///
/// 这台设备还没被问过通知权限时（模拟器刚建好就是这种）先弹系统的授权框，允许以后一样落到云吞的通知页——
/// iOS 只给问过的 App 建这一页。行右侧写着系统里的状态。连开发栈（设置页上的“通知”行由核心给）：
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
        executionTimeAllowance = 300
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        UITestBackend.enableStateProbe(to: app)
        try UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    func testSystemNotificationsEntryOpensWontonsOwnNotificationPage() {
        // 落点没成立时记一条失败也接着走，后面的状态断言才有机会说明问题。
        continueAfterFailure = true
        reachSettings()
        let notifications = app.buttons.matching(NSPredicate(format: "label == %@", "Notifications"))
        for _ in 0..<3 where hittable(notifications) == nil { app.swipeUp() }
        XCTAssertNotNil(hittable(notifications), "设置页里没有“通知”\n\(app.debugDescription)")
        hittable(notifications)?.tap()

        let entry = poll(uiWait, until: { row("System Notifications") })
        XCTAssertNotNil(entry, "通知页里没有“系统通知”\n\(app.debugDescription)")
        let value = poll(uiWait, until: { rowValue(entry, excluding: "System Notifications") })
        XCTAssertEqual(value, "Not allowed", "没问过权限时行右侧应写“未允许”\n\(app.debugDescription)")
        shot("01-notification-settings")
        entry?.tap()

        // 没问过权限：系统先弹授权框。按钮跟着模拟器语言走，不认字：两个按钮里第二个是“允许”。
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        if allow.waitForExistence(timeout: uiWait) {
            shot("02-permission-alert")
            allow.tap()
        } else {
            shot("02-no-permission-alert")
        }

        let prefs = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        let front = prefs.wait(for: .runningForeground, timeout: 30)
        shot("03-system-settings")
        XCTAssertTrue(front, "点“系统通知”没把系统设置带到前台\n\(app.debugDescription)")
        // 先断言入口自己落到了云吞的通知页。27.1 模拟器上深链到不了的话在这里记一条失败，
        // 再自己把系统设置走到云吞的页上，后面的状态断言照跑。
        let landed = waitUntil(uiWait, { isOnNotificationPage(prefs) })
        XCTAssertTrue(landed, "入口没落到云吞的通知页\n\(prefs.debugDescription)")
        if !landed {
            shot("03b-system-settings-home")
            if !isOnWontPage(prefs) { openWontPage(in: prefs) }
            let notificationEntry = poll(uiWait, until: { row(prefs, labels: ["Notifications", "通知"]) })
            XCTAssertNotNil(notificationEntry, "云吞的页里没有“通知”这一行\n\(prefs.debugDescription)")
            notificationEntry?.tap()
        }
        XCTAssertTrue(waitUntil(uiWait, { isOnNotificationPage(prefs) }), "没进云吞的通知页\n\(prefs.debugDescription)")
        shot("04-wonton-notification-page")
    }

    /// 冷启动连上家里的服务器，走完或跳 Compose 引导，从侧边栏进设置。
    private func reachSettings() {
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        for _ in 0..<8 where !sidebar.waitForExistence(timeout: uiWait) {
            if UITestBackend.skipKeyHandoff(app) { continue }
            let step = ["Skip", "Done", "Next"].map { app.buttons[$0].firstMatch }.first { $0.exists && $0.isEnabled }
            step?.tap()
        }
        XCTAssertTrue(sidebar.waitForExistence(timeout: networkWait), "没到主界面")
        UITestBackend.settleKeyHandoff(app)
        guard let settings = UITestSidebar.row(app, labels: ["Settings", "设置"]) else {
            return XCTFail("侧边栏里没有设置\n\(app.debugDescription)")
        }
        settings.tap()
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

    /// 系统设置首页 → App 列表 → 云吞 → 云吞的页。
    private func openWontPage(in prefs: XCUIApplication) {
        // 在别的子页里：先退回首页。
        for _ in 0..<4 {
            guard let back = backButton(in: prefs) else { break }
            back.tap()
        }
        var appsRow: XCUIElement?
        for _ in 0..<8 where appsRow == nil {
            appsRow = row(prefs, labels: ["App", "Apps"])
            if appsRow == nil { prefs.swipeUp() }
        }
        XCTAssertNotNil(appsRow, "系统设置首页没有 App 列表入口\n\(prefs.debugDescription)")
        appsRow?.tap()
        var appCell: XCUIElement?
        for _ in 0..<10 where appCell == nil {
            appCell = wontonCell(in: prefs)
            if appCell == nil { prefs.swipeUp() }
        }
        XCTAssertNotNil(appCell, "App 列表里没有云吞\n\(prefs.debugDescription)")
        appCell?.tap()
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
