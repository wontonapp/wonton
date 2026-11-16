import XCTest

/// 同一条真实后端路径用于材质修前/修后取证，不以首屏 fixture 代替配对后的主入口。
@MainActor
final class LiquidGlassUITests: XCTestCase {
    private var app: XCUIApplication!
    private static let sidebar = ["侧边栏", "Sidebar"]
    private static let back = ["返回", "Back"]
    private static let newChat = ["新建聊天", "New Chat"]

    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 300
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
        try UITestBackend.passPairingCode(to: app)
    }

    func testMainEntrancesStayReachable() throws {
        app.launch()
        try reachChatList()
        try assertListAndCapture("01-chat-list")

        try tap(Self.newChat)
        XCTAssertNotNil(button(Self.back, timeout: 10), app.debugDescription)
        capture("02-new-chat")
        try tap(Self.back)
        try assertListAndCapture("03-chat-list-returned")

        try enterBotRoom()
        capture("04-chat-room")
        let field = try XCTUnwrap(composer(timeout: 10), app.debugDescription)
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        capture("05-chat-keyboard")
        try tap(Self.back)
        try assertListAndCapture("06-chat-list-from-room")

        try tap(Self.sidebar)
        XCTAssertNotNil(button(["全部", "All"], timeout: 10), app.debugDescription)
        capture("07-sidebar-open")
        try tap(["收起侧边栏", "Collapse sidebar"])
        try assertListAndCapture("08-sidebar-closed")

        try tap(Self.sidebar)
        try tap(["设置", "Settings"])
        XCTAssertNotNil(button(["关闭", "Close"], timeout: 10), app.debugDescription)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        capture("09-settings")
        try verifySettingsSheetTransition()
        try tap(["关闭", "Close"])
        try assertListAndCapture("10-settings-closed")

        try tap(Self.sidebar)
        try tap(["全部", "All"])
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertNotNil(button(["设置", "Settings"], timeout: 10), app.debugDescription)
        XCTAssertNil(button(Self.newChat, timeout: 0), app.debugDescription)
        capture("11-my-apps")
    }

    private func verifySettingsSheetTransition() throws {
        let friends = app.buttons.matching(NSPredicate(
            format: "label BEGINSWITH %@ OR label BEGINSWITH %@", "好友", "Friends"
        )).firstMatch
        let deadline = Date().addingTimeInterval(10)
        while !friends.isHittable, Date() < deadline {
            app.scrollViews.firstMatch.swipeUp()
        }
        XCTAssertTrue(friends.isHittable, app.debugDescription)
        friends.tap()
        try tap(["加好友", "Add Friend"])
        XCTAssertNotNil(button(["扫对方的码", "Scan Their Code"], timeout: 10), app.debugDescription)
        capture("12-add-friend-sheet")
        try tap(["扫对方的码", "Scan Their Code"])
        let scannerField = app.textFields.matching(NSPredicate(
            format: "placeholderValue == %@ OR placeholderValue == %@", "粘贴对方的码", "Paste Their Code"
        )).firstMatch
        XCTAssertTrue(scannerField.waitForExistence(timeout: 10), app.debugDescription)
        capture("13-friend-scanner")
        try tap(["关闭", "Close"])
        XCTAssertNotNil(button(["加好友", "Add Friend"], timeout: 10), app.debugDescription)
        XCTAssertFalse(scannerField.exists, app.debugDescription)
        try tap(Self.back)
        XCTAssertNotNil(button(["关闭", "Close"], timeout: 10), app.debugDescription)
    }

    private func reachChatList() throws {
        let deadline = Date().addingTimeInterval(90)
        let onboarding = ["跳过", "Skip", "进入聊天", "Go to Chat", "完成", "Done", "下一步", "Next"]
        while Date() < deadline {
            if app.descendants(matching: .any)["pairing-refusal"].exists {
                XCTFail("配对被拒绝\n\(app.debugDescription)")
                return
            }
            if UITestBackend.skipKeyHandoff(app) { continue }
            if let step = button(onboarding, timeout: 0) {
                step.tap()
            } else if button(Self.sidebar, timeout: 0) != nil, button(Self.newChat, timeout: 0) != nil {
                UITestBackend.settleKeyHandoff(app)
                return
            } else if composer(timeout: 0) != nil, let back = button(Self.back, timeout: 0) {
                back.tap()
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        capture("00-chat-list-not-ready")
        XCTFail("配对和引导后未到聊天列表\n\(app.debugDescription)")
    }

    private func assertListAndCapture(_ name: String) throws {
        XCTAssertNotNil(button(Self.sidebar, timeout: 10), app.debugDescription)
        XCTAssertNotNil(button(Self.newChat, timeout: 10), app.debugDescription)
        capture(name)
    }

    private func enterBotRoom() throws {
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if let row = app.buttons.allElementsBoundByIndex.first(where: {
                $0.label.hasPrefix("Hermes") && $0.isHittable
            }) {
                row.tap()
                XCTAssertNotNil(composer(timeout: 10), app.debugDescription)
                return
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        XCTFail("开发栈没有可进入的 Hermes 房间\n\(app.debugDescription)")
    }

    private func composer(timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let field = app.textViews.allElementsBoundByIndex.first(where: {
                ["消息", "Message"].contains($0.label) && $0.isHittable
            }) { return field }
            if Date() >= deadline { return nil }
            Thread.sleep(forTimeInterval: 0.3)
        } while true
    }

    /// 模态页和底层可能有同名按钮：必须选实际可点击的匹配，不用 firstMatch 点到被盖住的那个。
    private func button(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for label in labels {
                if let button = app.buttons.matching(identifier: label).allElementsBoundByIndex.first(where: {
                    $0.isEnabled && $0.isHittable
                }) { return button }
            }
            if Date() >= deadline { return nil }
            Thread.sleep(forTimeInterval: 0.3)
        } while true
    }

    private func tap(_ labels: [String]) throws {
        try XCTUnwrap(button(labels, timeout: 10), "入口不可点击：\(labels)\n\(app.debugDescription)").tap()
    }

    private func capture(_ name: String) {
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = "\(name)-accessibility"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
    }
}
