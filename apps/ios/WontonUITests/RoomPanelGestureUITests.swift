import XCTest

/// 房间顶栏按钮打开面板（ADR-0052）：不要后端，用 `WONTON_UI_FAKE` 的假数据。
/// 高消息贴底后继续上滑不打开面板；按钮打开整页，网页栏缘收回和聊天条拖动保留。
final class RoomPanelGestureUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// 回到底部后继续上滑只滚消息；顶栏按钮仍可打开整页面板。
    func testScrollingLeavesPanelClosedAndButtonOpensFull() {
        launch()
        let field = enterRoom()
        note("landed \(scrollState)")
        XCTAssertTrue(scrollState.contains("pin=false"), "落点在靠前的一条上，不该贴底：\(scrollState)")
        shot("01-landed")
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch)
        Thread.sleep(forTimeInterval: 2)
        note("after-back-to-bottom \(scrollState)")
        XCTAssertTrue(scrollState.contains("pin=true"), "点了回到底部没贴底：\(scrollState)")
        shot("02-bottom")

        let last = text(startingWith: "Fixture last line")
        XCTAssertTrue(last.waitForExistence(timeout: 5), "回到底部后看不到最后一条的最后一行：\(scrollState)")
        XCTAssertLessThanOrEqual(last.frame.maxY, field.frame.minY + 1,
                                 "最后一行在输入框下面：\(last.frame) 输入框 \(field.frame) \(scrollState)")
        XCTAssertGreaterThan(last.frame.minY, app.frame.midY, "最后一行不在底部：\(last.frame) \(scrollState)")

        pullUp(above: field)
        XCTAssertFalse(app.otherElements["chatPanel"].exists, "到底后上滑打开了面板")
        XCTAssertFalse(app.buttons["panelOpen"].exists, "房间顶栏还有打开面板的按钮")
        openPanelFromProfile()
        let panel = app.otherElements["chatPanel"]
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "房间按钮没有打开面板")
        XCTAssertTrue(wait(5) { panel.value as? String == "full" }, "有页面的房间按钮没有直接进入整页")
        shot("03-button-full")
    }

    /// 房间的面板到了整页：网页里面起手的下拉归网页，面板不动；从网页顶边附近起手才退回第 1 级。
    func testFullPanelOnlyReturnsFromThePageEdge() {
        launch()
        let field = enterRoom()
        let safeTop = app.buttons["Back"].frame.minY
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch)
        Thread.sleep(forTimeInterval: 2)
        openPanelFromProfile()
        XCTAssertTrue(app.webViews.staticTexts["Fixture player"].waitForExistence(timeout: 20), "面板网页没加载出来")
        XCTAssertTrue(wait(5) { !self.onScreen(field) }, "有页面的房间按钮没有直接进入整页")
        shot("11-room-full")

        dragDown(fromY: app.frame.height * 0.45)
        XCTAssertFalse(wait(2) { self.onScreen(field) }, "网页里面起手的下拉把整页面板拖回了第 1 级")
        shot("12-still-full")

        dragDown(fromY: safeTop + 12)
        XCTAssertTrue(wait(5) { self.onScreen(field) }, "从网页顶边起手的下拉没回第 1 级")
        shot("13-back-to-peek")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)),
                   withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(wait(5) { !self.onScreen(field) }, "露出的聊天条拖动不能回到整页")
    }

    /// 换级照 ADR-0052 的门槛：走够 pushDistance / pullDistance，或者够 flickDistance 又够快；都不够就弹回原级。
    /// 距离门槛用差一截的慢拖，速度门槛另用差一截的短甩：都够不到自己的门槛，过不过只由速度解释。
    func testPanelChangesLevelOnlyPastDistanceOrFlick() {
        launch()
        let field = enterRoom()
        let safeTop = app.buttons["Back"].frame.minY
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch)
        Thread.sleep(forTimeInterval: 2)
        openPanelFromProfile()
        XCTAssertTrue(app.webViews.staticTexts["Fixture player"].waitForExistence(timeout: 20), "面板网页没加载出来")
        XCTAssertTrue(wait(5) { self.panelLevel == "full" }, "有页面的房间按钮没有直接进入整页")
        print("手势速度 slow", XCUIGestureVelocity.slow, "default", XCUIGestureVelocity.default,
              "fast", XCUIGestureVelocity.fast)

        // 整页：页顶慢拖差一截 pullDistance 弹回；行程更短、只靠速度的快甩就收起。
        drag(fromY: safeTop + 12, by: Self.below(Self.pull), velocity: .slow)
        XCTAssertTrue(wait(3) { self.panelLevel == "full" }, "差一截 pullDistance 的慢拖换了级：\(panelLevel)")
        drag(fromY: safeTop + 12, by: Self.flickTravel(under: Self.pull), velocity: Self.flickVelocity)
        XCTAssertTrue(wait(5) { self.panelLevel == "peek" }, "够快的短甩没回第 1 级：\(panelLevel)")
        shot("21-peek-by-flick")

        // 第 1 级：聊天条往上差一截 pushDistance 弹回；同样行程的快甩就进整页。
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: -Self.below(Self.push),
             velocity: .slow, press: Self.stripPressDuration)
        XCTAssertTrue(wait(3) { self.panelLevel == "peek" }, "差一截 pushDistance 的慢拖进了整页：\(panelLevel)")
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: -Self.flickTravel(under: Self.push),
             velocity: Self.stripFlickVelocity, press: Self.stripPressDuration)
        XCTAssertTrue(wait(5) { self.panelLevel == "full" }, "够快的短甩没进整页：\(panelLevel)")

        // 走够距离就不看速度：整页往下拖够 pullDistance 退到第 1 级，再往上拖够 pushDistance 回整页。
        drag(fromY: safeTop + 12, by: Self.above(Self.pull), velocity: .slow)
        XCTAssertTrue(wait(5) { self.panelLevel == "peek" }, "慢拖走够 pullDistance 没回第 1 级：\(panelLevel)")
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: -Self.above(Self.push),
             velocity: .slow, press: Self.stripPressDuration)
        XCTAssertTrue(wait(5) { self.panelLevel == "full" }, "慢拖走够 pushDistance 没进整页：\(panelLevel)")

        // 第 1 级往下：差一截 pullDistance 弹回，够快就收起，走够距离也收起。
        drag(fromY: safeTop + 12, by: Self.above(Self.pull), velocity: .slow)
        XCTAssertTrue(wait(5) { self.panelLevel == "peek" }, "慢拖走够 pullDistance 没退回第 1 级：\(panelLevel)")
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: Self.below(Self.pull),
             velocity: .slow, press: Self.stripPressDuration)
        XCTAssertTrue(wait(3) { self.panelLevel == "peek" }, "差一截 pullDistance 的慢拖收起了面板：\(panelLevel)")
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: Self.flickTravel(under: Self.pull),
             velocity: Self.stripFlickVelocity, press: Self.stripPressDuration)
        XCTAssertTrue(wait(5) { self.panelLevel == "closed" }, "够快的短甩没收起：\(panelLevel)")
        XCTAssertTrue(onScreen(field), "收起以后没回到房间")

        // 收起后回到了房间：重开要走一遍头像 → 资料页 → 打开面板。
        openPanelFromProfile()
        XCTAssertTrue(wait(5) { self.panelLevel == "full" }, "重开没进整页：\(panelLevel)")
        // 重开落在整页：往下慢拖走够 pullDistance 先到第 1 级（页顶下拉区；0.3 屏高已经进了网页里面），
        // 再从露出的聊天条往下拖够收起。
        drag(fromY: safeTop + 12, by: Self.above(Self.pull), velocity: .slow)
        XCTAssertTrue(wait(5) { self.panelLevel == "peek" }, "慢拖走够 pullDistance 没回第 1 级：\(panelLevel)")
        drag(fromX: app.frame.midX, fromY: app.frame.height * 0.3, by: Self.above(Self.pull),
             velocity: .slow, press: Self.stripPressDuration)
        XCTAssertTrue(wait(5) { self.panelLevel == "closed" }, "慢拖走够 pullDistance 没收起：\(panelLevel)")
        XCTAssertTrue(onScreen(field), "收起以后没回到房间")
    }
}

extension RoomPanelGestureUITests {
    private func launch() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    /// 进假数据里的房间，返回输入框。
    /// 房间面板的入口在 Bot 资料页（ADR-0052）：点顶栏头像进资料页，再点“打开面板”。
    /// 打开时产品会把资料页退掉（`openPanelFromProfile` 里 `showsBotProfile = false`），收起后回到的是房间，重开要再走一遍。
    private func openPanelFromProfile() {
        tap(app.buttons["roomTopBarAvatar"])
        let entry = app.buttons["botProfilePanelOpen"]
        XCTAssertTrue(entry.waitForExistence(timeout: 20), "Bot 资料页没有打开面板的按钮\n\(app.debugDescription)")
        for _ in 0..<4 where !entry.isHittable {
            app.swipeUp()
        }
        entry.tap()
    }

    /// 从屏幕 `(x, startY)` 处竖直拖 `distance`（正数朝下）：整页时走页顶的下拉区（x 靠左，避开右上角的胶囊），
    /// 第 1 级时走露出的聊天条（x 取中间，按压也更短，见 stripPressDuration）。
    private func drag(
        fromX x: CGFloat = 48, fromY startY: CGFloat, by distance: CGFloat,
        velocity: XCUIGestureVelocity, press: TimeInterval = 0.05
    ) {
        let from = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: startY))
        from.press(forDuration: press, thenDragTo: from.withOffset(CGVector(dx: 0, dy: distance)),
                   withVelocity: velocity, thenHoldForDuration: 0)
    }

    /// 面板现在在哪一级：`chatPanel` 的辅助功能值；收起以后它从界面树里拿掉，值也是 closed。
    private var panelLevel: String {
        let panel = app.otherElements["chatPanel"]
        return panel.exists ? (panel.value as? String ?? "?") : "closed"
    }

    // 换级门槛和快甩速度：`Wonton/Design/Generated/WontonTokens.swift` 的 gesture.pullDistance(96)、
    // pushDistance(64)、flickDistance(32)、flickVelocity(900)。界面测试编不进 App 模块，抄一份，改了令牌这里跟着改。
    private static let pull: CGFloat = 96
    private static let push: CGFloat = 64
    private static let flick: CGFloat = 32
    /// 快甩速度（点/秒）：XCUITest 的 `.fast` 实际只按 750 点/秒注入（XCTest 日志里的注入速度），过不了 900 的门槛，所以显式给。
    /// 页顶那条边是 UIPan 按事件时间量的，1_200 就量得到 1_200 上下；露出的聊天条是 SwiftUI 的 DragGesture，把按下到抬手
    /// 整段时间也算进速度、量到的大约只有五分之一，得把按下压到 0.01 秒再试。
    private static let flickVelocity: XCUIGestureVelocity = 1_200
    private static let stripFlickVelocity: XCUIGestureVelocity = 3_000
    /// 露出的聊天条上按下到开始移动的间隔：SwiftUI 把它算进速度的分母。页顶那条边不受影响，用默认的 0.05。
    private static let stripPressDuration: TimeInterval = 0.01

    /// 门槛下面一截：注入的拖动先被 touch slop 吃掉一截，留 8 点余量；再短 XCUITest 会当成点按。
    private static func below(_ threshold: CGFloat) -> CGFloat { threshold - 8 }
    /// 门槛上面一截。
    private static func above(_ threshold: CGFloat) -> CGFloat { threshold + 32 }
    /// 快甩的行程：贴着距离门槛下面 2 点（不会被当成距离过；注入的拖动只会被 slop 吃短，不会变长），
    /// 同时比 flickDistance 至少多 8 点，速度是唯一的解释；按压很短，量到的速度才够过门槛。
    private static func flickTravel(under threshold: CGFloat) -> CGFloat { max(flick + 8, threshold - 2) }

    private func enterRoom() -> XCUIElement {
        tap(app.staticTexts["Echo"])
        let field = app.textViews["Message"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "没进房间\n\(app.debugDescription)")
        XCTAssertTrue(text(startingWith: "Fixture message 3").waitForExistence(timeout: 10), "时间线没出来")
        return field
    }

    /// 从消息区慢速上滑，确认长距离滚动不会再触发面板入口。
    private func pullUp(above field: XCUIElement) {
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let start = origin.withOffset(CGVector(dx: app.frame.midX, dy: field.frame.minY - 40))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -160)),
                    withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 消息的文字和时间合在一个元素里，按开头找。
    private func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 靠左起手：整页时右上角到中间是胶囊（公网网页的胶囊带域名，更宽），手指落在它上面到不了网页。
    private func dragDown(fromY start: CGFloat) {
        let from = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 48, dy: start))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 220)),
                   withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 输入框还在屏幕里：整页面板把房间整个推出屏幕。
    private func onScreen(_ field: XCUIElement) -> Bool {
        field.exists && field.frame.maxY > app.frame.minY && field.frame.minY < app.frame.maxY
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    /// App 里 `BottomPin` 的自述。
    private var scrollState: String { app.staticTexts["roomScroll"].label }

    private func tap(_ element: XCUIElement) {
        XCTAssertTrue(element.waitForExistence(timeout: 20), "找不到 \(element)\n\(app.debugDescription)")
        element.tap()
    }

    private func note(_ text: String) {
        let attachment = XCTAttachment(string: text)
        attachment.name = text
        attachment.lifetime = .keepAlways
        add(attachment)
        print("NOTE \(text)")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
