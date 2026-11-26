import XCTest

/// 面板里的网页和面板手势怎么分（ADR-0052）：不要后端，用 `WONTON_UI_FAKE` 的假数据。
///
/// 假数据的面板里是一张盖着全屏固定浮层的本机网页，右上角一个关闭按钮（照 Navidrome 的手机播放器摆）：
/// 浮层开着时底下的页面一直停在顶上，按“滚到顶了接着拉”分不出手指是在拖页面还是拖面板。
final class PanelGestureUITests: XCTestCase {
    private var app: XCUIApplication!
    private var safeTop: CGFloat = 0

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// 有页面的主面板从按钮直接进入整页；网页正文和顶边的拖动都不重新触发开工手势。
    func testMainPanelButtonOpensFullAndKeepsPageDrags() {
        launch()
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 300)),
                    withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertFalse(panel.exists, "聊天列表拖动打开了面板")
        openMainPanel()
        dragDown(fromY: app.frame.height * 0.4)
        XCTAssertTrue(waitLevel("full", timeout: 2) && page.exists, "网页正文拖动改变了整页面板")
        dragDown(fromY: pageTop + 12)
        XCTAssertTrue(waitLevel("full", timeout: 2), "主面板整页的网页顶边拖动改变了级别")
        shot("03-full")
    }

    /// 整页面板上胶囊浮在网页右上角：最大的辅助字号和深色下，挨着它的网页按钮照样点得到，胶囊也还在。
    func testPageButtonBesideTheCapsuleStaysTappable() {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        openMainPanel()
        XCTAssertTrue(waitLevel("full", timeout: 5), "没到第 2 级\n\(app.debugDescription)")
        shot("11-full-largest-type")

        let close = app.webViews.buttons["Fixture close"]
        XCTAssertTrue(close.waitForExistence(timeout: 5), "网页的关闭按钮不在")
        close.tap()
        XCTAssertTrue(app.webViews.buttons["Fixture closed"].waitForExistence(timeout: 5),
                      "网页右上角的按钮点不到：\(close.frame)")
        XCTAssertTrue(waitLevel("full", timeout: 1) && capsuleHittable, "点网页按钮把面板关了")
        shot("12-page-button-tapped")
    }

    /// 面板拉开过再收起：收起的面板看不见，旁白也不该摸到它——面板、胶囊的“更多”“关闭”、网页都不在可访问树里。
    /// 按 `exists` 认：旁白读的是可访问树，点不点得到（`isHittable`）是另一回事。
    func testClosedPanelLeavesNothingForVoiceOver() {
        launch()
        openMainPanel()
        XCTAssertTrue(capsuleHittable && app.buttons["pageMore"].firstMatch.exists, "整页面板没有胶囊")
        let close = panel.buttons.matching(identifier: "pageClose").allElementsBoundByIndex.first { $0.isHittable }
        close?.tap()
        // 收起后聊天页回到原位，顶栏按钮回到状态栏下沿。
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(wait(5) { sidebar.isHittable && abs(sidebar.frame.minY - self.safeTop) < 1 }, "点胶囊的关闭没收起面板")
        shot("21-closed")

        let any = app.descendants(matching: .any)
        XCTAssertTrue(wait(3) { !self.panel.exists }, "收起的面板还在可访问树里：\(String(describing: panel.value))")
        XCTAssertEqual(any.matching(identifier: "pageClose").count, 0, "收起的面板里胶囊的关闭还在可访问树里")
        XCTAssertEqual(any.matching(identifier: "pageMore").count, 0, "收起的面板里胶囊的更多还在可访问树里")
        XCTAssertEqual(any.matching(identifier: "panelDots").count, 0, "收起的面板里小圆点还在可访问树里")
        XCTAssertEqual(app.webViews.count, 0, "收起的面板里网页还在可访问树里\n\(app.debugDescription)")
        XCTAssertFalse(page.exists, "收起的面板里网页的元素还在可访问树里")

        // 再从按钮打开，旁白又摸得到。空面板第一下只到 PEEK，交给统一入口拉满。
        openMainPanel()
        XCTAssertTrue(page.exists && capsuleHittable, "重新打开后网页或胶囊不在可访问树里")
    }

    /// 面板里的网页在放声音（网页播放器）：收起面板不打断它，过一阵再拉开，声音一直在放。
    /// 收起的面板不在界面上，网页得留着接着跑。
    func testClosingThePanelKeepsPageAudioPlaying() {
        launch()
        openMainPanel()
        app.webViews.buttons["Fixture play"].tap()
        XCTAssertTrue(wait(15) { (self.pageAudio?.seconds ?? 0) >= 1 && self.pageAudio?.playing == true },
                      "网页的声音没放起来：\(audioStatus.label)")
        guard let before = pageAudio else { return XCTFail("读不到网页的播放状态") }

        panel.buttons.matching(identifier: "pageClose").allElementsBoundByIndex.first { $0.isHittable }?.tap()
        XCTAssertTrue(wait(5) { !self.panel.exists }, "点胶囊的关闭没收起面板")
        Thread.sleep(forTimeInterval: 10)

        app.buttons["panelOpen"].tap()
        XCTAssertTrue(waitLevel("full", timeout: 5) && audioStatus.waitForExistence(timeout: 10), "收起后按钮打不开")
        // 收起的 10 秒里声音停过的话，播放位置最多只多出收起前后那两三秒。
        XCTAssertTrue(wait(5) { (self.pageAudio?.seconds ?? 0) >= before.seconds + 9 },
                      "面板收起的时候网页的声音停了：收起前 \(before)，再拉开 \(audioStatus.label)")
        XCTAssertEqual(pageAudio?.playing, true, "再拉开时网页的声音没在放：\(audioStatus.label)")
        XCTAssertEqual(pageAudio?.pauses, before.pauses, "面板收起的时候网页的声音被暂停过：\(audioStatus.label)")
        shot("31-still-playing")
    }
}

extension PanelGestureUITests {
    private func launch(arguments: [String] = []) {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + arguments
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 20), "没进主界面\n\(app.debugDescription)")
        // 顶栏按钮紧挨着状态栏下沿：UI 测试读不到安全区，拿它当状态栏的高度。
        safeTop = sidebar.frame.minY
    }

    /// `panelDots` 在 iOS 26.5 上落在各圆点叶节点，不能再向下查 descendants。
    private func openMainPanel() {
        let open = app.buttons["panelOpen"]
        XCTAssertTrue(open.waitForExistence(timeout: 5) && open.isHittable, "面板入口按钮不可达")
        open.tap()
        XCTAssertTrue(wait(5) {
            self.panel.exists && (self.panel.value as? String == "peek" || self.panel.value as? String == "full")
        }, "按钮没有打开面板")
        let startedPeek = panel.value as? String == "peek"
        if startedPeek, app.buttons["panelAdd"].exists {
            app.buttons["panelAdd"].tap()
            XCTAssertTrue(app.textFields["https://"].waitForExistence(timeout: 5), "空面板加号没有打开编辑页")
            return XCTFail("fixture 面板为空；已进入添加页面，网页手势场景需要真实 Player 页面")
        }
        if !page.exists {
            let dots = panel.descendants(matching: .any).matching(identifier: "panelDots")
            let player = dots.matching(NSPredicate(format: "label == %@", "Player")).firstMatch
            if player.waitForExistence(timeout: 5) {
                XCTAssertTrue(player.isHittable, "Player 圆点不可点\n\(app.debugDescription)")
                player.tap()
            } else {
                XCTAssertEqual(dots.count, 0, "有分页圆点，但缺少 fixture 的 Player 圆点\n\(app.debugDescription)")
            }
        }
        // 单页没有圆点，仍必须证明当前选中的是实际可加载的 fixture 网页。
        XCTAssertTrue(page.waitForExistence(timeout: 20), "fixture 没有就绪的 Player 网页；检查单页及回环服务\n\(app.debugDescription)")
        if startedPeek {
            dragDown(fromY: pageTop + 12)
            XCTAssertTrue(waitLevel("full", timeout: 5), "PEEK 里的真实网页没有从栏缘进入 FULL")
        } else {
            XCTAssertTrue(waitLevel("full", timeout: 2), "切页改变了整页面板的级别")
        }
    }

    private var panel: XCUIElement { app.otherElements["chatPanel"] }

    /// 假数据网页上的那层浮层。
    private var page: XCUIElement { app.webViews.staticTexts["Fixture player"] }

    /// 假数据网页自己报的播放状态，如 `Fixture audio playing 12 pauses 0`。
    private var audioStatus: XCUIElement {
        app.webViews.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture audio")).firstMatch
    }

    private struct PageAudio {
        let playing: Bool
        let seconds: Int
        let pauses: Int
    }

    private var pageAudio: PageAudio? {
        guard audioStatus.exists else { return nil }
        let words = audioStatus.label.split(separator: " ")
        guard words.count == 6, let seconds = Int(words[3]), let pauses = Int(words[5]) else { return nil }
        return PageAudio(playing: words[2] == "playing", seconds: seconds, pauses: pauses)
    }

    /// 网页露出来的顶边：主面板的网页框从屏幕顶上开始，状态栏那一截不算（`launch` 里按顶栏按钮的位置记下）。
    private var pageTop: CGFloat { max(app.webViews.firstMatch.frame.minY, safeTop) }

    /// 靠左起手：整页时右上角到中间是胶囊（公网网页的胶囊带域名，更宽），手指落在它上面到不了网页。
    private func dragDown(fromY start: CGFloat) {
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let from = origin.withOffset(CGVector(dx: 48, dy: start))
        let end = from.withOffset(CGVector(dx: 0, dy: 220))
        from.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 当前这一页的胶囊点得到。每一页各有一个胶囊，旁边那页的在屏幕外。
    private var capsuleHittable: Bool {
        panel.buttons.matching(identifier: "pageClose").allElementsBoundByIndex.contains { $0.exists && $0.isHittable }
    }

    /// 面板自己报的级别（`closed`、`peek`、`full`）。
    private func waitLevel(_ level: String, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if panel.value as? String == level { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
