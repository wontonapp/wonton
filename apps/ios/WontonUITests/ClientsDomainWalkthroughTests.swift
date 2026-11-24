import XCTest

/// “HTTPS 和域名”设置页的实走（ADR-0089）：对开发栈的 Pebble 配一张证书，截下表单、签发和 CAA 建议。
/// 需要带 Pebble 的开发栈（`mini-walkthrough.sh up` 起的栈就带）和一条配对码。
final class ClientsDomainWalkthroughTests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func waitAnyButton(_ labels: [String], timeout: TimeInterval = 20) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists && element.isHittable { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 配对完成：配对页收了、有跳过的引导按钮或列表。
    private func waitPaired(timeout: TimeInterval) -> Bool {
        let anyElement = app.descendants(matching: .any)
        let deadline = Date().addingTimeInterval(timeout)
        var sawCode = false
        while Date() < deadline {
            if anyElement["pairing-refusal"].exists { return false }
            if anyElement["pairing-code"].exists {
                // 配对页还在：等它收起来（这个家里已经过了引导，成功后不会停在任何可等的页面上）。
                sawCode = true
            } else if sawCode
                || app.buttons.matching(UITestSidebar.button).firstMatch.exists
                || app.buttons["Skip"].exists
            {
                return true
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return false
    }

    private func passOnboarding() {
        for _ in 0..<60 {
            // 配了新设备、家里没有别的已登录设备时，会停在“等其他设备交接密钥”页：先等它超时，再点 Later。
            if let later = waitAnyButton(["Later", "稍后"], timeout: 1) { later.tap(); continue }
            if let alone = waitAnyButton(["I don't have my other device", "我没有另一台设备"], timeout: 1) {
                alone.tap()
                continue
            }
            if UITestBackend.skipKeyHandoff(app) { continue }
            if UITestSidebar.waitForMain(app, timeout: 1) { return }
            if let skip = waitAnyButton(["Skip"], timeout: 2), skip.isEnabled { skip.tap(); continue }
            if let done = waitAnyButton(["Done"], timeout: 2), done.isEnabled { done.tap(); continue }
            if let next = waitAnyButton(["Next"], timeout: 2), next.isEnabled { next.tap(); continue }
        }
    }

    /// 元素可能在一屏下面：往上滑到能点为止。
    @discardableResult
    private func scrollTo(_ element: XCUIElement, timeout: TimeInterval = 20) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if element.exists && element.isHittable { return true }
            app.swipeUp()
        }
        return element.exists && element.isHittable
    }

    /// 行会被设置页的重取重画：等它出现后再现查一次，点不到就重来。
    @discardableResult
    private func tapRow(_ label: String, timeout: TimeInterval = 20) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let element = waitAnyButton([label], timeout: 2), element.exists {
                usleep(400_000)
                let fresh = app.buttons[label].firstMatch
                // 行刚出现时 frame 还是空的，tap/isHittable 会抛 “Activation point invalid”：
                // 只等 frame 长出来（读 frame 不抛），等不到就往上滚。
                if fresh.exists, fresh.frame.height > 0 {
                    fresh.tap()
                    return true
                }
                app.swipeUp()
            }
        }
        return false
    }

    /// 开侧边栏并点设置；抽屉的行走得比打开慢时多试几次。
    private func openSettings(timeout: TimeInterval = 90) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let settings = UITestSidebar.row(app, labels: ["Settings"], timeout: 3) {
                settings.tap()
                return true
            }
            let open = app.buttons.matching(UITestSidebar.button).firstMatch
            if open.exists && open.isHittable { open.tap() }
        }
        return false
    }

    /// 回到（或重新打开）域名页；已经不在设置里就从头开。
    private func ensureDomainPage() -> Bool {
        if app.staticTexts["Certificate"].exists { return true }
        guard openSettings() else { return false }
        return tapRow("HTTPS and domain", timeout: 20)
    }

    private func type(_ text: String, into element: XCUIElement) {
        XCTAssertTrue(scrollTo(element), "找不到输入框 \(element)")
        element.tap()
        element.typeText(text)
    }

    func testConfigurePebbleCertificate() throws {
        XCTAssertTrue(waitPaired(timeout: 120), "配对没完成：\(app.debugDescription)")
        passOnboarding()
        // 引导里可能还盖着“等其他设备交接”的层：没到列表就再点一次 Later，到了才开设置。
        if !UITestSidebar.waitForChatList(app, timeout: 30) {
            waitAnyButton(["Later", "稍后"], timeout: 10)?.tap()
            XCTAssertTrue(UITestSidebar.waitForChatList(app, timeout: 60), "没到聊天列表")
        }
        // 刚配对完时设置可能还是配对前的旧快照（没有入口）：退回去重开，等它自己刷过来。
        var entered = false
        for _ in 0..<12 where !entered {
            guard openSettings() else { continue }
            entered = tapRow("HTTPS and domain", timeout: 15)
            if !entered {
                _ = try? app.navigationBars.buttons.firstMatch.tap()
                usleep(1_000_000)
            }
        }
        XCTAssertTrue(entered, "设置里没有 HTTPS 和域名入口")
        XCTAssertTrue(app.textFields["clientsDomainSuffix"].waitForExistence(timeout: 30), "域名页没打开")
        shot("01-empty")

        // 栈上存过后缀时输入框带着旧值，再打会拼上去（太长的名字 Pebble 的 DNS 查询会溢出）：
        // 长按调出编辑菜单选“全选”再替换。删除键的键值直接 typeText 是发给 App 的，删不掉。
        let suffixField = app.textFields["clientsDomainSuffix"]
        XCTAssertTrue(scrollTo(suffixField), "没有域名字段")
        suffixField.tap()
        suffixField.press(forDuration: 1.2)
        let selectAll = app.menuItems["Select All"].exists
            ? app.menuItems["Select All"]
            : app.buttons["Select All"]
        if selectAll.waitForExistence(timeout: 3) {
            selectAll.tap()
        }
        suffixField.typeText("example.test")
        // 键盘盖住下半页会吞掉滑动手势：先收起键盘，“其他”在懒加载列表的更下面。
        if app.keyboards.count > 0 { app.typeText("\n") }
        var other: XCUIElement?
        for _ in 0..<4 where other == nil {
            app.swipeUp()
            other = waitAnyButton(["Other", "其他"], timeout: 15)
        }
        guard let other else { return XCTFail("没有“其他”：\(app.debugDescription)") }
        XCTAssertTrue(scrollTo(other), "“其他”点不到")
        usleep(300_000)
        other.tap()
        type("httpreq", into: app.textFields["clientsDomainOtherCode"])
        type(
            "HTTPREQ_ENDPOINT=http://httpreq:8484",
            into: app.textFields["clientsDomainOtherCredentials"]
        )
        shot("02-filled")

        let terms = app.switches["clientsDomainTerms"].exists
            ? app.switches["clientsDomainTerms"]
            : app.staticTexts["I accept the Let's Encrypt subscriber agreement"]
        XCTAssertTrue(scrollTo(terms), "没有订户协议")
        if (terms.value as? String) != "1" {
            terms.tap()
        }
        shot("03-terms")

        let save = app.buttons["clientsDomainSave"]
        XCTAssertTrue(scrollTo(save), "没有保存")
        save.tap()
        // 保存成功就回设置页；再进来看着签发，签完了看证书。
        XCTAssertTrue(tapRow("HTTPS and domain", timeout: 60), "保存后没回设置页：\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Certificate"].waitForExistence(timeout: 30), "域名页没重开")
        // 页面不自己刷新证书：没签完就退出去再进来，重新拉一次。
        let issued = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Expires'")).firstMatch
        var issuedHit = false
        for _ in 0..<10 where !issuedHit {
            if issued.waitForExistence(timeout: 15) {
                issuedHit = true
                break
            }
            if app.staticTexts["Issuing"].exists {
                _ = scrollTo(app.staticTexts["Issuing"])
                shot("04-issuing")
            }
            // 退出去再进来拉新状态；退多了一步（回到聊天列表）就重开设置。
            _ = try? app.navigationBars.buttons.firstMatch.tap()
            usleep(500_000)
            XCTAssertTrue(ensureDomainPage(), "回不去域名页")
        }
        XCTAssertTrue(issuedHit, "证书没签出来：\(app.debugDescription)")
        _ = scrollTo(issued)
        shot("05-issued")

        // CAA 建议整条可复制。
        XCTAssertTrue(app.staticTexts["CAA"].waitForExistence(timeout: 20), "没有 CAA 一节")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'CAA 0 issue'")).firstMatch.exists,
            "CAA 没有给出记录"
        )
        shot("06-caa")
        app.swipeUp()
        shot("07-lan")

        // deSEC 的注册和限定令牌说明在默认收起的“详情”里：选中 deSEC 再展开（不发请求）。
        app.swipeDown()
        app.swipeDown()
        app.swipeDown()
        if let desecRow = waitAnyButton(["deSEC"], timeout: 10) {
            desecRow.tap()
            if let desec = waitAnyButton(["deSEC token"], timeout: 10) {
                desec.tap()
                shot("08-desec")
            }
        }
    }
}
