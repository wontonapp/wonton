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

    /// 主聊天页的面板在第 1 级：网页里面起手的下拉归网页，面板不动；从网页顶边附近起手才进第 2 级。
    func testPagePullOnlyStartsNearThePageEdge() {
        launch()
        openMainPanel()
        shot("01-peek")

        dragDown(fromY: app.frame.height * 0.4)
        XCTAssertFalse(waitLevel("full", timeout: 2), "网页里面起手的下拉把面板拖进了第 2 级")
        XCTAssertTrue(waitLevel("peek", timeout: 1) && page.exists, "网页里面起手的下拉把面板拖走了")
        shot("02-still-peek")

        dragDown(fromY: pageTop + 12)
        XCTAssertTrue(waitLevel("full", timeout: 5), "从网页顶边起手的下拉没进第 2 级\n\(app.debugDescription)")
        shot("03-full")
    }

    /// 整页面板上胶囊浮在网页右上角：最大的辅助字号和深色下，挨着它的网页按钮照样点得到，胶囊也还在。
    func testPageButtonBesideTheCapsuleStaysTappable() {
        XCUIDevice.shared.appearance = .dark
        defer { XCUIDevice.shared.appearance = .light }
        launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        openMainPanel()
        dragDown(fromY: pageTop + 12)
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
        XCTAssertTrue(capsuleHittable && app.buttons["pageMore"].firstMatch.exists, "第 1 级没有胶囊")
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

        // 再拉开，旁白又摸得到。
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        low.press(forDuration: 0.05, thenDragTo: low.withOffset(CGVector(dx: 0, dy: 160)),
                  withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(waitLevel("peek", timeout: 5), "收起后再拉不开")
        XCTAssertTrue(page.waitForExistence(timeout: 20) && capsuleHittable, "再拉开后网页或胶囊不在可访问树里")
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

        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        low.press(forDuration: 0.05, thenDragTo: low.withOffset(CGVector(dx: 0, dy: 160)),
                  withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(waitLevel("peek", timeout: 5) && audioStatus.waitForExistence(timeout: 10), "收起后再拉不开")
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
        app.launch()
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 20), "没进主界面\n\(app.debugDescription)")
        // 顶栏按钮紧挨着状态栏下沿：UI 测试读不到安全区，拿它当状态栏的高度。
        safeTop = sidebar.frame.minY
    }

    /// 聊天列表到顶后往下拉到第 1 级，翻到加载得出来的那一页（假数据的第一页没有隧道，一直是骨架）。
    private func openMainPanel() {
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        low.press(forDuration: 0.05, thenDragTo: low.withOffset(CGVector(dx: 0, dy: 160)),
                  withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "列表到顶后往下拉没有面板")
        XCTAssertTrue(waitLevel("peek", timeout: 5), "面板不在第 1 级：\(String(describing: panel.value))")
        // 第 1 级横着滑是翻页。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.4))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.4)),
                   withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(page.waitForExistence(timeout: 20), "面板网页没加载出来\n\(app.debugDescription)")
        XCTAssertTrue(waitLevel("peek", timeout: 2), "翻页把面板带离了第 1 级：\(String(describing: panel.value))")
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
