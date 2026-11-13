import XCTest

/// Bot 资料（ADR-0044）的走屏：改名、静态头像、4 个状态头像，以及聊天顶栏随 Bot 变化。
///
/// 全新安装会先落在引导页（`OnboardingGate` 顶替整页），所以先走完引导或跳过。
/// 每一步都留截图附件；外观（浅色/深色）由外部 `simctl ui` 设定，名字写在附件里。
final class BotProfileUITests: XCTestCase {
    private var app: XCUIApplication!

    private var appearance: String {
        ProcessInfo.processInfo.environment["APPEARANCE"] ?? "light"
    }

    override func setUpWithError() throws {
        continueAfterFailure = true
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "\(appearance)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// 等某个标识或标签出现（按 exists 判断，不要求可点）。
    @discardableResult
    private func waitFor(_ queries: [XCUIElementQuery], _ label: String, timeout: TimeInterval = 20) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for query in queries {
                let element = query[label].firstMatch
                if element.exists { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 引导页：跳过优先，跳过不了就按下一步/完成，直到侧边栏按钮出现。
    /// 等一组候选按钮里任意一个出现（同一个按钮的不同语言写法）。
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

    /// 等元素可点再点（选择器弹出动画里点击会报 not hittable）。
    private func tapWhenHittable(_ element: XCUIElement) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !element.isHittable {
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    /// 应用页上 Bot 的图标：按钮本身没有标签，标签在里面的文字上。
    private func tile() -> XCUIElement? {
        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            for name in ["回声", "Echo"] {
                let byLabel = app.buttons.containing(.staticText, identifier: name).firstMatch
                if byLabel.exists { return byLabel }
                let text = app.staticTexts[name].firstMatch
                if text.exists { return text }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 相册选择器里点第一张图：优先按元素，取不到就按坐标点网格左上第一格。
    private func tapFirstPhoto() {
        let cell = app.cells.firstMatch
        if cell.waitForExistence(timeout: 15) {
            tapWhenHittable(cell)
            return
        }
        // 选择器占屏幕下半，缩略图是够大的图片元素（frame 属性不能进 NSPredicate，改成取回来后筛）。
        let bottom = app.windows.firstMatch.frame.height * 0.4
        let thumbnail = app.images.allElementsBoundByIndex.first { $0.frame.width > 80 && $0.frame.minY > bottom }
        if let thumbnail {
            tapWhenHittable(thumbnail)
            return
        }
        // 兜底：选择器占屏幕下半，左上第一格大约在这里。
        let window = app.windows.firstMatch.frame
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.16, dy: 0.62)).tap()
        sleep(2)
    }

    /// 按屏幕坐标长按：元素上的长按会被可访问性激活吃掉（变成点按），改用 app 的坐标。
    private func pressScreen(_ element: XCUIElement, _ seconds: TimeInterval) {
        let window = app.windows.firstMatch.frame
        let frame = element.frame
        let offset = CGVector(dx: frame.midX / window.width, dy: frame.midY / window.height)
        app.coordinate(withNormalizedOffset: offset).press(forDuration: seconds)
    }

    /// 底栏没了：入口在侧边栏，开抽屉找那一行（ADR-0095）。
    private func tabButton(_ labels: [String]) -> XCUIElement? {
        UITestSidebar.row(app, labels: labels)
    }

    /// 主界面到了：根页面左上角有侧边栏按钮。
    @discardableResult
    private func waitForTabs(timeout: TimeInterval) -> Bool {
        UITestSidebar.waitForMain(app, timeout: timeout)
    }

    private func passOnboarding() {
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            if waitForTabs(timeout: 1) { return }
            // 最后一页的「跳过」是禁用的，要看「完成」。
            if let skip = waitAnyButton(["Skip", "跳过"], timeout: 2), skip.isEnabled {
                skip.tap()
                continue
            }
            if let done = waitAnyButton(["Done", "完成"], timeout: 2), done.isEnabled {
                done.tap()
                continue
            }
            if let next = waitAnyButton(["Next", "下一步"], timeout: 2), next.isEnabled {
                next.tap()
                continue
            }
        }
    }

    func testProfileFlows() throws {
        continueAfterFailure = false
        // 冷启动先恢复连接和引导状态：等到「跳过」或侧边栏按钮任一个。
        // 冷启动先恢复连接；引导页和主界面二选一，别只等一个。
        let start = waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成"], timeout: 25)
            ?? waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成"], timeout: 60)
        XCTAssertNotNil(start, "既没到主界面也没到引导页")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(waitForTabs(timeout: 240), "没回到主界面")
        // 落在哪个标签不确定，统一去应用页。
        // 引导结束后落在哪个标签不确定，一定要切到应用页再找卡片（别的页也有叫「回声」的元素）。
        tabButton(["All Apps", "全部应用", "全部應用程式", "すべてのアプリ"])?.tap()
        XCTAssertTrue(app.buttons["应用商店"].waitForExistence(timeout: 15), "不在应用页")
        XCTAssertTrue(waitForTabs(timeout: 10), "侧边栏消失了")
        shot("00-launcher")

        // 长按 Echo → 详情。
        // 长按可能被当成点按（进私聊），所以每次按压后看结果：出菜单就用，进了私聊就返回重来。
        var details: XCUIElement?
        for attempt in 0..<3 where details == nil {
            let echo = tile()
            XCTAssertNotNil(echo, "应用页没有 Echo 卡片")
            guard let echo else { break }
            echo.press(forDuration: attempt == 0 ? 1.5 : 2.0)
            details = waitFor([app.buttons], "详情", timeout: 4) ?? waitFor([app.buttons], "Details", timeout: 4)
            if details == nil, app.descendants(matching: .any)["roomTopBarAvatar"].waitForExistence(timeout: 3) {
                if let back = waitAnyButton(["Back", "返回"], timeout: 3) { back.tap() } else { app.navigationBars.buttons.firstMatch.tap() }
                UITestSidebar.waitForMain(app, timeout: 5)
            }
        }
        print("HIER_AFTER_LONGPRESS:\(app.debugDescription)")
        XCTAssertNotNil(details, "长按菜单没有详情")
        details?.tap()

        XCTAssertTrue(app.buttons["botProfileName"].waitForExistence(timeout: 20), "没进资料节")
        shot("01-profile")

        // 改名。
        let name = app.buttons["botProfileName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10), "资料节没有名字")
        name.tap()
        let field = app.textFields["botProfileNameField"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "名字没进入编辑")
        field.tap()
        field.press(forDuration: 1.2)
        if app.menuItems["全选"].exists { app.menuItems["全选"].tap() } else if app.menuItems["Select All"].exists { app.menuItems["Select All"].tap() }
        field.typeText("EchoBot")
        shot("02-name-editing")
        app.buttons["botProfileSave"].tap()
        XCTAssertTrue(app.buttons["botProfileName"].waitForExistence(timeout: 20), "改名后没回到名字显示")
        XCTAssertEqual(app.buttons["botProfileName"].label, "EchoBot", "名字没改成 EchoBot")
        shot("03-renamed")

        // 静态头像（系统相册选择器，选第一张）。
        app.buttons["botProfileChooseStatic"].tap()
        tapFirstPhoto()
        XCTAssertTrue(app.buttons["botProfileRemoveStatic"].waitForExistence(timeout: 60), "静态头像没设上")
        shot("04-static-avatar")

        // 四个状态头像：两个用图片，第三个用视频，第四个留空。
        for slot in ["idle", "working", "waiting"] {
            let cell = app.buttons["botProfileState.\(slot)"]
            XCTAssertTrue(cell.waitForExistence(timeout: 10), "缺状态格：\(slot)")
            cell.tap()
            // 已经有头像时先弹操作表，选「换」。
            if let choose = waitAnyButton(["Choose", "选择"], timeout: 3) { choose.tap() }
            tapFirstPhoto()
            _ = app.buttons["botProfileState.\(slot)"].waitForExistence(timeout: 60)
            shot("05-state-\(slot)")
        }
        shot("06-all-states")

        // 顶栏：回列表（这一页导航栏是隐藏的，返回是自绘按钮），进和 EchoBot 的私聊。
        if let back = waitAnyButton(["Back", "返回"], timeout: 5) {
            back.tap()
        } else {
            app.tabBars.buttons.firstMatch.tap()
        }
        tabButton(["Chats", "聊天"])?.tap()
        let row = app.buttons["EchoBot"].firstMatch.exists ? app.buttons["EchoBot"].firstMatch : app.staticTexts["EchoBot"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 20), "聊天列表没有 EchoBot 行")
        row.tap()
        XCTAssertTrue(app.descendants(matching: .any)["roomTopBarAvatar"].waitForExistence(timeout: 20), "聊天顶栏没有 Bot 头像")
        shot("07-topbar")
    }
}
