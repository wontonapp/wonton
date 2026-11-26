import XCTest

/// 胶囊菜单的“返回”按网页历史判定（同 Android #960）：上一步还是同一页（重定向、pushState/replaceState 记录）时
/// 不再盲 goBack，直接关掉这一页；上一步会换文档、只换片段时照常后退。不要后端：假数据的主面板里钉着一张本机
/// fixture 网页（`ui_fake_room` 的 `/`），按钮 push 再 replace 出一条地址还是本页的历史。只换片段的一步在 Android
/// 的判定里不算同一页（`PageHistoryTest.fragmentStepIsAnotherPage`），这里照同样的口径。
final class PanelBackUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// 同一地址的历史步（重定向回本页）：点“返回”应当关页（面板项摘掉，面板还开着），不是按了没反应。浅色深色各走一遍。
    func testSameDocumentBackClosesThePage() throws {
        try walk(style: "Light", prefix: "light")
        app.terminate()
        try walk(style: "Dark", prefix: "dark")
    }

    /// 会换文档的历史步：点“返回”照常后退，页面还在，不关页。
    func testCrossDocumentBackGoesBackInHistory() throws {
        try launch()
        try openPlayerInPanel()
        let cover = app.links["Fixture cover"].firstMatch
        XCTAssertTrue(cover.waitForExistence(timeout: 20), "fixture 页面里没有换文档的链接\n\(app.debugDescription)")
        cover.tap()
        XCTAssertTrue(waitPath("/corner-cover"), "点链接没有换文档")
        openCapsuleMenu()
        XCTAssertTrue(back.waitForExistence(timeout: 10), "换文档的历史步上没有“返回”")
        shot("cross-document")
        back.tap()
        XCTAssertTrue(waitPath("/"), "换文档的历史步点“返回”没有退回去")
        XCTAssertTrue(playerDot.exists, "换文档的历史步点“返回”把页面关了")
    }

    /// 只换片段的一步（hash 路由）：地址不同，点“返回”照常后退，页面不关。
    func testFragmentOnlyStepGoesBackInHistory() throws {
        try launch()
        try openPlayerInPanel()
        let fragment = app.links["Fixture fragment"].firstMatch
        XCTAssertTrue(fragment.waitForExistence(timeout: 20), "fixture 页面里没有片段链接\n\(app.debugDescription)")
        fragment.tap()
        XCTAssertTrue(waitPath("/#/album"), "点片段链接没有换片段")
        openCapsuleMenu()
        XCTAssertTrue(back.waitForExistence(timeout: 10), "只换片段的历史步上没有“返回”")
        shot("fragment")
        back.tap()
        XCTAssertTrue(waitPath("/"), "只换片段的历史步点“返回”没有退回去")
        XCTAssertTrue(playerDot.exists, "只换片段的历史步点把页面关了")
    }

    // MARK: - 摆台

    private func walk(style: String, prefix: String) throws {
        try launch(style: style)
        try openPlayerInPanel()
        let same = app.buttons["Fixture same document"].firstMatch
        XCTAssertTrue(same.waitForExistence(timeout: 20), "fixture 页面里没有同址按钮\n\(app.debugDescription)")
        // push 到 /same 再 replace 回 /：多出来的一条历史地址还是本页，走回去只会回到原处。
        same.tap()
        openCapsuleMenu()
        XCTAssertTrue(back.waitForExistence(timeout: 10), "同一地址的历史步上没有“返回”")
        shot("\(prefix)-01-back-menu")
        back.tap()
        XCTAssertTrue(waitGone(playerDot), "同一地址的历史步点“返回”没有关页")
        XCTAssertFalse(waitPath("/", timeout: 5), "关页后 fixture 页还在")
        XCTAssertNotEqual(app.otherElements["chatPanel"].value as? String, "closed", "关页把整个面板也关了")
        shot("\(prefix)-02-closed")
    }

    private func launch(style: String? = nil) throws {
        app = XCUIApplication()
        app.launchArguments += [
            "-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES",
        ]
        // 系统外观切换在这代模拟器运行时不一定生效：钉住主题（同 `WontonTheme` 的截图开关），浅色深色各截一张。
        app.launchEnvironment["WONTON_SHOT_SCHEME"] = style?.lowercased() ?? "light"
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        UITestBackend.enableStateProbe(to: app)
        UITestBackend.launch(app)
        let arrived = waitAnyButton(["Skip", "跳过", "Next", "下一步"], timeout: 60) != nil
            || app.scrollViews.firstMatch.waitForExistence(timeout: 2)
        XCTAssertTrue(arrived, "既没到聊天列表，也没到引导页\n\(app.debugDescription)")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 20), "没到聊天页")
    }

    /// 假数据的主面板里钉着两页（Budget 和本机的 Player fixture），点亮 Player 的点。
    private func openPlayerInPanel() throws {
        openPanel()
        XCTAssertTrue(playerDot.waitForExistence(timeout: 10), "面板里没有 Player 页\n\(app.debugDescription)")
        playerDot.tap()
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 20), "面板网页没加载出来")
        // 有页面时顶栏按钮直接打开整页；万一停在半截，再从网页顶边往下拉。
        if !waitFull(timeout: 1) {
            pullPageDown()
            XCTAssertTrue(waitFull(), "面板没拉到整页")
        }
    }

    /// 面板顶部的小圆点，标签是页面标题。
    private var playerDot: XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Player")).firstMatch
    }

    private func openPanel() {
        let button = app.buttons.matching(identifier: "panelOpen").allElementsBoundByIndex
            .first { $0.exists && $0.isHittable }
        XCTAssertNotNil(button, "没有打开面板的按钮\n\(app.debugDescription)")
        button?.tap()
    }

    /// 从面板网页的顶边附近往下拉：页面滚到顶后接着拉，面板换到整页（ADR-0052）。
    private func pullPageDown() {
        let top = max(app.webViews.firstMatch.frame.minY + 12, 70)
        let inside = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 48, dy: top))
        let outward = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85))
        inside.press(forDuration: 0.05, thenDragTo: outward, withVelocity: .slow, thenHoldForDuration: 0)
    }

    private func waitFull(timeout: TimeInterval = 5) -> Bool {
        let panel = app.otherElements["chatPanel"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if panel.exists, panel.value as? String == "full" { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return false
    }

    private func openCapsuleMenu() {
        let more = app.buttons.matching(identifier: "pageMore").firstMatch
        XCTAssertTrue(more.waitForExistence(timeout: 10), "胶囊菜单没出来\n\(app.debugDescription)")
        more.tap()
    }

    /// 菜单里的“返回”。页面换了文档、或上一页是同一页时它都在。
    private var back: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
    }

    /// fixture 页面自己写着当前地址（`#path`）和第几次加载（`#loads`）。
    private func waitPath(_ path: String, timeout: TimeInterval = 20) -> Bool {
        waitText("Fixture path \(path)", timeout: timeout)
    }

    private func waitText(_ label: String, timeout: TimeInterval = 20) -> Bool {
        let text = app.staticTexts.matching(NSPredicate(format: "label == %@", label)).firstMatch
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if text.exists { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return false
    }

    private static let onboardingButtons = [
        "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成",
        "Next", "下一步", "Skip", "跳过",
    ]

    /// 主人在这台核心上走过引导时不再弹，所以引导按钮没出现也算走完。
    private func passOnboarding() {
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            guard let button = waitAnyButton(Self.onboardingButtons, timeout: 3) else { return }
            button.tap()
        }
    }

    @discardableResult
    private func waitAnyButton(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(min(timeout, 10))
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists, element.isEnabled, element.isHittable { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 0.5)
        }
        return nil
    }

    private func tapAnyButton(_ labels: [String], timeout: TimeInterval) {
        guard let button = waitAnyButton(labels, timeout: timeout) else {
            return XCTFail("没等到按钮：\(labels)\n\(app.debugDescription)")
        }
        button.tap()
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
