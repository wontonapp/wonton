import XCTest

/// “HTTPS 和域名”设置页的实走（ADR-0089）：对开发栈的 Pebble 配一张证书，截下表单、签发和 CAA 建议。
/// 需要带 Pebble 的开发栈（`mini-walkthrough.sh up` 起的栈就带）和一条配对码。
final class ClientsDomainWalkthroughTests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 全新模拟器要把引导整条走完（每页一个 Skip），默认 300 秒不够。
        executionTimeAllowance = 600
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

    /// 等配对和引导都真的走完：配对页收起、四步引导点到底（4/4 的 Install 或 Skip）、
    /// 出现聊天列表才算数。只看“有 Skip”会在 1/4 就返回，重启后又回到引导。
    /// 配对页自己也有 Skip，所以见到过码、出现交接页或侧栏之后才动引导按钮。
    private func waitPaired(timeout: TimeInterval) -> Bool {
        let anyElement = app.descendants(matching: .any)
        let deadline = Date().addingTimeInterval(timeout)
        var sawCode = false
        while Date() < deadline {
            allowNotifications()
            if anyElement["pairing-refusal"].exists { return false }
            // 配对页在引导层下面时 `pairing-code` 一直在树里，不能因为它就说“还在配对”。
            if anyElement["pairing-code"].exists { sawCode = true }
            // 交接层盖在列表上时列表元素照样查得到，先把它关掉再谈“到了下一屏”。
            let handoff = app.buttons["key-handoff-skip"].exists || app.buttons["Try again"].exists
            if handoff, skipHandoff() { continue }
            // 配对后的聊天备份页也会盖住列表：它有返回键，先关掉。
            if app.scrollViews["chat-backup"].exists, UITestBackend.skipKeyHandoff(app) { continue }
            if UITestSidebar.chatListVisible(app) { return true }
            if sawCode || app.buttons.matching(UITestSidebar.button).firstMatch.exists {
                if tapFirst([
                    "Go to Chat", "进入聊天", "Install these apps", "安装这些应用", "Next", "下一步", "Done", "完成",
                    "Skip", "跳过"
                ]) {
                    _ = app.wait(for: .runningForeground, timeout: 0.5)
                    continue
                }
                if let later = waitAnyButton(["Later", "稍后"], timeout: 1) {
                    later.tap()
                    continue
                }
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return false
    }

    /// 交接页的“我没有另一台设备”：压着时 `isHittable` 可能为假，按坐标点。
    private func skipHandoff() -> Bool {
        let skip = app.buttons["key-handoff-skip"]
        guard skip.exists else { return false }
        if skip.isHittable {
            skip.tap()
        } else if skip.frame.height > 0 {
            skip.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        } else {
            return false
        }
        _ = app.wait(for: .runningForeground, timeout: 0.5)
        return true
    }

    /// 新装的 App 会弹系统通知授权，盖在引导页上把“跳过交接”挡住；两边语言都认。
    private func allowNotifications() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for label in ["Allow", "允许"] {
            let button = springboard.buttons[label]
            if button.exists {
                button.tap()
                return
            }
        }
    }

    /// 点第一个在场且可点的按钮；不算等待，先把当前的页推走。
    /// 压着状态栏或底栏、`isHittable` 为假时按坐标点，别让整条引导卡在最后一页。
    private func tapFirst(_ labels: [String]) -> Bool {
        for label in labels {
            let button = app.buttons[label].firstMatch
            guard button.exists && button.isEnabled else { continue }
            if button.isHittable {
                button.tap()
                return true
            }
            if button.frame.height > 0 {
                button.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
                return true
            }
        }
        return false
    }

    private func passOnboarding() {
        for _ in 0..<200 {
            allowNotifications()
            if skipHandoff() { continue }
            if tapFirst(["Skip", "跳过", "Done", "完成", "Next", "下一步", "Go to Chat", "进入聊天", "Install these apps", "安装这些应用"]) { continue }
            if UITestBackend.skipKeyHandoff(app) { continue }
            if UITestSidebar.waitForMain(app, timeout: 1) { return }
            if let later = waitAnyButton(["Later", "稍后"], timeout: 1) { later.tap(); continue }
            if let alone = waitAnyButton(["I don't have my other device", "我没有另一台设备"], timeout: 1) {
                alone.tap()
                continue
            }
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
    /// 点设置里的一行：行在列表下方、还没滚进可视区时，`tap()` 会因
    /// “Activation point invalid”直接判失败。所以先滚到 `isHittable` 再点；
    /// 滚过头了再往回滚。
    private func tapRow(_ label: String, timeout: TimeInterval = 20) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        var swipes = 0
        while Date() < deadline {
            let row = app.buttons[label].firstMatch
            if row.exists {
                if row.isHittable {
                    row.tap()
                    return true
                }
                if swipes < 6 {
                    app.swipeUp()
                    swipes += 1
                } else {
                    app.swipeDown()
                }
                usleep(400_000)
                continue
            }
            usleep(300_000)
        }
        return false
    }

    /// 点导航栏返回；没有导航栏（已经在列表）就别点，空查询的 tap 会记一条失败。
    private func goBack() {
        let back = app.navigationBars.buttons.firstMatch
        if back.exists { back.tap() }
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

    /// 回到（或重新打开）域名页：设置页的快照要等，30 秒内反复试。
    private func ensureDomainPage() -> Bool {
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if app.staticTexts["Certificate"].exists { return true }
            if UITestSidebar.chatListVisible(app) {
                _ = openSettings()
            }
            if tapRow("HTTPS and domain", timeout: 5) {
                return true
            }
            usleep(500_000)
        }
        return app.staticTexts["Certificate"].exists
    }

    private func type(_ text: String, into element: XCUIElement) {
        XCTAssertTrue(scrollTo(element), "找不到输入框 \(element)")
        element.tap()
        element.typeText(text)
    }

    func testConfigurePebbleCertificate() throws {
        try enterDomainPage()
        configureOtherProvider()
        saveAndWaitIssued()
        assertCaaAndLan()
        showDesecDetails()
    }

    /// 配对、走完引导和交接页、重启一版、进设置打开域名页。
    private func enterDomainPage() throws {
        allowNotifications()
        XCTAssertTrue(waitPaired(timeout: 120), "配对没完成：\(app.debugDescription)")
        passOnboarding()
        // 引导里可能还盖着“等其他设备交接”的层：没到列表就再点一次 Later，到了才开设置。
        if !UITestSidebar.waitForChatList(app, timeout: 30) {
            waitAnyButton(["Later", "稍后"], timeout: 10)?.tap()
            XCTAssertTrue(UITestSidebar.waitForChatList(app, timeout: 60), "没到聊天列表")
        }
        // 刚配对完时设置可能还是配对前的旧快照（没有入口）：重启一版 App 强制重建，再从聊天列表走。
        app.terminate()
        app.launch()
        passOnboarding()
        if !UITestSidebar.waitForChatList(app, timeout: 30) {
            waitAnyButton(["Later", "稍后"], timeout: 10)?.tap()
            XCTAssertTrue(UITestSidebar.waitForChatList(app, timeout: 60), "重启后没到聊天列表")
        }
        // 仍没入口就退回去重开，等它自己刷过来。
        var entered = false
        for _ in 0..<12 where !entered {
            guard openSettings() else { continue }
            entered = tapRow("HTTPS and domain", timeout: 15)
            usleep(2_000_000)
            if !entered {
                goBack()
                usleep(1_000_000)
            }
        }
        XCTAssertTrue(entered, "设置里没有 HTTPS 和域名入口")
        XCTAssertTrue(app.textFields["clientsDomainSuffix"].waitForExistence(timeout: 30), "域名页没打开")
        shot("01-empty")
    }

    /// 填域名、“其他”服务商和凭据，勾上订户协议。
    private func configureOtherProvider() {
        // 栈上存过后缀时输入框带着旧值；tap 会把光标落在点中的字中间，delete 只删左边，会凑出怪名字。
        // 先用 ⌘A 全选直接替换，值不对再从末尾按键盘删除键重来，最后校验值本身。
        let suffixField = app.textFields["clientsDomainSuffix"]
        XCTAssertTrue(scrollTo(suffixField), "没有域名字段")
        suffixField.tap()
        suffixField.typeKey("a", modifierFlags: .command)
        suffixField.typeText("example.test")
        usleep(300_000)
        if (suffixField.value as? String) != "example.test" {
            for _ in 0..<160 {
                app.keys["delete"].tap()
            }
            suffixField.typeText("example.test")
            XCTAssertEqual(suffixField.value as? String, "example.test", "域名没清干净")
        }
        // 键盘不专门收：滑动照样能滚列表（typeText 的回车会去找键盘上的 return 键，键位标识不稳）。
        // “其他”在懒加载列表的更下面，往上滑。
        var other: XCUIElement?
        for _ in 0..<8 where other == nil {
            app.swipeUp(velocity: .slow)
            other = waitAnyButton(["Other", "其他"], timeout: 8)
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
            ? app.switches["clientsDomainTerms".replacingOccurrences(of: "clients", with: "clients")].exists
                ? app.switches["clientsDomainTerms"]
                : app.staticTexts["I accept the Let's Encrypt subscriber agreement"]
            : app.staticTexts["I accept the Let's Encrypt subscriber agreement"]
        XCTAssertTrue(scrollTo(terms), "没有订户协议")
        if (terms.value as? String) != "1" {
            terms.tap()
        }
        shot("03-terms")
    }

    /// 保存并等证书签发；页面不自己刷新，没签完就退出去再进来。
    private func saveAndWaitIssued() {
        let save = app.buttons["clientsDomainSave"]
        XCTAssertTrue(scrollTo(save), "没有保存")
        save.tap()
        XCTAssertTrue(tapRow("HTTPS and domain", timeout: 60), "保存后没回设置页：\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Certificate"].waitForExistence(timeout: 30), "域名页没重开")
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
            goBack()
            usleep(500_000)
            XCTAssertTrue(ensureDomainPage(), "回不去域名页")
        }
        XCTAssertTrue(issuedHit, "证书没签出来：\(app.debugDescription)")
        _ = scrollTo(issued)
        shot("05-issued")
    }

    /// CAA 建议整条可复制；`*.lan` 记录也在这一页。
    private func assertCaaAndLan() {
        XCTAssertTrue(app.staticTexts["CAA"].waitForExistence(timeout: 20), "没有 CAA 一节")
        // 整条记录要能直接照抄：带当前 CA 身份（开发栈是 pebble）、本机账号和 dns-01，缺一条都到不了“已限定”。
        let caa = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS 'CAA 0 issue \"pebble;' AND label CONTAINS 'accounturi=' AND label CONTAINS 'validationmethods=dns-01'")
        ).firstMatch
        XCTAssertTrue(caa.exists, "CAA 记录不完整：\(app.debugDescription)")
        shot("06-caa")
        app.swipeUp()
        // 局域网记录要写出具体地址，不是“this host's LAN address”占位；地址段随主机，
        // 只要求是个点分四段地址。
        let lan = app.staticTexts.matching(
            NSPredicate(
                format: "label BEGINSWITH '*.lan.' AND label MATCHES '.* A ([0-9]{1,3}\\.){3}[0-9]{1,3}$'"
            )
        ).firstMatch
        XCTAssertTrue(lan.exists, "局域网记录没写地址：\(app.debugDescription)")
        shot("07-lan")
    }

    /// deSEC 的注册和限定令牌说明在默认收起的“详情”里：选中 deSEC 再展开（不发请求）。
    private func showDesecDetails() {
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
