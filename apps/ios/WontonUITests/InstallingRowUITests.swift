import XCTest

/// 聊天列表里的安装行（ADR-0067）的走屏：配对、从商店装一个 Agent，截下阶段和进度、失败、详情和重试。
///
/// 装的是 `PLUGIN` 环境变量给的插件（默认 Claude Code）：它在首次启动时要下载东西，会在检查期限里失败，
/// 这样一次运行里既有进度也有失败。外观（浅色/深色）由外部 `simctl ui` 设定，名字写在附件里。
final class InstallingRowUITests: XCTestCase {
    private var app: XCUIApplication!

    private var appearance: String { ProcessInfo.processInfo.environment["APPEARANCE"] ?? "light" }
    private var plugin: String {
        (ProcessInfo.processInfo.environment["PLUGIN"] ?? "Claude Code").replacingOccurrences(of: "%20", with: " ")
    }

    override func setUpWithError() throws {
        continueAfterFailure = true
        app = XCUIApplication()
        // 模拟器的系统语言是中文也行：钉成英文，标记和查询对得上。
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
    }

    private func shot(_ name: String) {
        let image = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: image)
        attachment.name = "\(appearance)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
        let directory = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("shots")
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("\(appearance)-\(name).png")
        try? image.pngRepresentation.write(to: file)
        print("SHOT:\(file.path)")
    }

    @discardableResult
    private func waitFor(_ queries: [XCUIElementQuery], _ label: String, timeout: TimeInterval = 20) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for query in queries where query[label].firstMatch.exists { return query[label].firstMatch }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    private func waitAnyButton(_ labels: [String], timeout: TimeInterval = 10) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    private func tapWhenHittable(_ element: XCUIElement) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !element.isHittable {
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        element.tap()
    }

    private func tabButton(_ labels: [String]) -> XCUIElement? {
        let height = app.windows.firstMatch.frame.height
        for label in labels {
            for element in app.buttons.matching(identifier: label).allElementsBoundByIndex
            where element.frame.minY > height * 0.7 {
                return element
            }
        }
        return nil
    }

    @discardableResult
    private func waitForTabs(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if tabButton(["Apps", "Chats", "应用", "聊天"]) != nil { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return false
    }

    /// 还在配对页时粘贴配对码，配不上就换下一个码。已经配上就什么都不做。
    private func pairIfNeeded(_ pairCodes: [String]) {
        let field = app.textFields.firstMatch
        guard field.waitForExistence(timeout: 20) else { return }
        for (index, code) in pairCodes.enumerated() {
            field.tap()
            UIPasteboard.general.string = code
            field.press(forDuration: 1.2)
            let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"] : app.menuItems["粘贴"]
            if paste.exists {
                paste.tap()
            } else {
                field.typeText(code)
            }
            print("PAIR_FIELD:\(String(describing: field.value))")
            shot("00-paired-\(index)")
            if let pair = waitAnyButton(["Pair", "配对"], timeout: 8), pair.isEnabled { pair.tap() }
            for _ in 0..<20 {
                if waitAnyButton(["Apps", "应用", "Skip", "跳过"], timeout: 1) != nil { return }
            }
            // 换下一个码：清空输入框。
            field.tap()
            field.press(forDuration: 1.2)
            if app.menuItems["Select All"].exists { app.menuItems["Select All"].tap() }
            if app.menuItems["全选"].exists { app.menuItems["全选"].tap() }
            field.typeText("\u{8}")
        }
    }

    private func passOnboarding() {
        for _ in 0..<12 {
            if waitForTabs(timeout: 1) { return }
            if let skip = waitAnyButton(["Skip", "跳过"], timeout: 2), skip.isEnabled {
                skip.tap()
                continue
            }
            if let done = waitAnyButton(["Done", "Go to chat", "完成", "去聊天"], timeout: 2), done.isEnabled {
                done.tap()
                continue
            }
            if let next = waitAnyButton(["Next", "下一步"], timeout: 2), next.isEnabled {
                next.tap()
                continue
            }
        }
    }

    /// 商店里这一行的 Get 按钮：按最靠近插件标题的那一个挑。
    private func getButton(for title: String) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(60)
        while Date() < deadline {
            let label = app.staticTexts[title].firstMatch
            if label.exists {
                let target = label.frame.midY
                let buttons = app.buttons.matching(NSPredicate(format: "label == %@ OR label == %@", "Get", "获取"))
                    .allElementsBoundByIndex
                let nearest = buttons.min { abs($0.frame.midY - target) < abs($1.frame.midY - target) }
                if let nearest, abs(nearest.frame.midY - target) < 80 { return nearest }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    func testInstallRow() throws {
        // 排队的机器上等锁很慢，最早的码可能已经过期：逐个试。
        let pairCodes = try UITestBackend.codes()
        try XCTSkipIf(pairCodes.isEmpty, "需要开发栈：没有 PAIRING_CODE")
        pairIfNeeded(pairCodes)
        XCTAssertTrue(
            waitAnyButton(["Apps", "应用"], timeout: 40) != nil
                || waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成"], timeout: 90) != nil,
            "既没到标签栏也没到引导页"
        )
        passOnboarding()
        XCTAssertTrue(waitForTabs(timeout: 300), "没回到主界面")

        tabButton(["Apps", "应用"])?.tap()
        let store = waitAnyButton(["Open the Store", "Store", "打开应用商店", "应用商店"], timeout: 25)
        XCTAssertNotNil(store, "应用页没有商店入口")
        store?.tap()
        shot("01-store")
        guard let get = getButton(for: plugin) else {
            XCTFail("商店里没有 \(plugin)")
            return
        }
        tapWhenHittable(get)

        let install = waitAnyButton(["Install", "安装"], timeout: 60)
        XCTAssertNotNil(install, "没有安装按钮")
        shot("02-confirm")
        install?.tap()
        _ = waitAnyButton(["Done", "完成"], timeout: 30)
        // 装到一半就能在聊天列表里看到这一行了：关掉完成页回聊天列表。
        let close = app.buttons["Close"]
        if close.exists { close.tap() }
        // 关掉确认页还在商店里：按返回退出商店，再切到聊天列表。
        if let back = waitAnyButton(["Back", "返回"], timeout: 8) { back.tap() }
        _ = waitAnyButton(["Apps", "应用"], timeout: 10)
        let chats = tabButton(["Chats", "聊天"])
        chats?.tap()
        // 商店里的任务行有一样的阶段和失败文字：断言真的切到了聊天标签，别拿它充数。
        XCTAssertEqual(chats?.isSelected, true, "没切到聊天标签（多半还在商店或别的页）")
        shot("02b-chat-list")

        // 阶段和进度。
        let running = waitFor([app.staticTexts], "Pulling images", timeout: 60)
        if running != nil {
            shot("03-pulling")
            for index in 0..<3 {
                shot("04-progress-\(index)")
                _ = app.wait(for: .runningForeground, timeout: 3)
            }
        }
        // 到列表时行多半已在等待或已经失败：短等一下等待态就截，别为等阶段白耗。
        _ = waitFor([app.staticTexts], "Waiting for the bot", timeout: 30)
        shot("05-waiting")

        // 失败：检查期限到了以后插件回滚，这一行给类别、详情和重试。
        let failed = waitFor([app.staticTexts], "Install failed: Your home server ran into a problem", timeout: 420)
        XCTAssertNotNil(failed, "没等到安装失败")
        shot("06-failed")
        if let details = waitAnyButton(["Details", "详情"], timeout: 10) {
            details.tap()
            shot("07-details")
        }
        if let retry = waitAnyButton(["Try Again", "重试"], timeout: 10) {
            retry.tap()
            XCTAssertNotNil(waitAnyButton(["Install", "安装"], timeout: 60), "重试没回到确认页")
            shot("08-retry-confirm")
        }
    }
}
