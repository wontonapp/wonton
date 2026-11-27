import XCTest

/// 走查（临时，不进提交）：回到聊天时没订阅上时间线，提示要用会话名字指路，不露房间 ID。
///
/// 订阅是异步的，回来那一瞬间发出去的动作会撞上还没订阅好的窗口；敲字（`SetTyping`）、回前台都会发。
/// 所以循环“敲一个字、切后台再回来”，撞上就让断言和截图留下。
final class ChatRoomNameToastUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 1100
    }

    func testNotSubscribedNamesTheConversation() throws {
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += [
            "-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES",
            // 列表一出现，App 就给第一个房间发一次打字（见 ChatListView.probeToastIfRequested）。
            "-wonton-toast-probe",
        ]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
        XCTAssertTrue(waitPaired(timeout: 180), "配对没完成\n\(app.debugDescription)")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 120), "没到聊天列表\n\(app.debugDescription)")
        // 这时那条时间线还没订阅上，核心回 NotReady，弹「现在还做不了：要先订阅…」。
        XCTAssertTrue(
            app.staticTexts["Not available right now"].waitForExistence(timeout: 60),
            "没弹出未就绪提示\n\(app.debugDescription)"
        )
        XCTAssertTrue(foundProblem(how: "probe"))
    }

    /// 弹着「Not available right now」时展开详情；是未订阅的提示就截图留下。
    private func foundProblem(how: String) -> Bool {
        guard app.staticTexts["Not available right now"].exists else {
            return false
        }
        let details = app.buttons["Details"].firstMatch
        if details.waitForExistence(timeout: 3) { details.tap() }
        let text = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "Subscribe to the timeline")
        ).firstMatch
        guard text.waitForExistence(timeout: 2) else {
            print("PROBLEM-OTHER\n\(app.debugDescription)")
            // 别的未就绪（比如还没登录）：关掉接着试。
            let ok = app.buttons["dialogOk"].firstMatch
            let close = app.buttons["composerRejectionClose"].firstMatch
            if ok.exists {
                ok.tap()
            } else if close.exists {
                close.tap()
            }
            return false
        }
        let label = text.label
        print("TOAST[\(how)]: \(label)")
        capture("not-subscribed-light")
        XCUIDevice.shared.appearance = .dark
        Thread.sleep(forTimeInterval: 1.5)
        capture("not-subscribed-dark")
        XCUIDevice.shared.appearance = .light
        print("TREE:\n\(app.debugDescription)")
        XCTAssertFalse(label.contains("!"), "提示里露了房间 ID：\(label)")
        XCTAssertTrue(label.contains("Subscribe to the timeline for"), "提示没用会话名字：\(label)")
        return true
    }

    /// 按无障碍标识找，不管它是哪种控件。
    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func botRoom(timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for name in ["Hermes", "回声", "Echo"] {
                let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", name)).firstMatch
                if row.exists, row.isHittable { return row }
            }
            let any = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chatRoom.")).firstMatch
            if any.exists, any.isHittable { return any }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    private func composer(timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(min(timeout, 10))
        while Date() < deadline {
            for label in ["Message", "消息", "訊息", "メッセージ"] {
                for view in app.textViews.allElementsBoundByIndex where view.label == label && view.exists {
                    return view
                }
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return nil
    }

    private func edgeSwipeBack() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
        start.press(
            forDuration: 0.05,
            thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
            withVelocity: .default,
            thenHoldForDuration: 0.2
        )
    }

    private func waitPaired(timeout: TimeInterval) -> Bool {
        let any = app.descendants(matching: .any)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if any["pairing-refusal"].exists { return false }
            let rendered = app.scrollViews.firstMatch.exists
                || app.buttons["Skip"].exists || app.buttons["跳过"].exists
            if rendered && !any["pairing-code"].exists { return true }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return false
    }

    private func passOnboarding() {
        let buttons = [
            "Go to Chat", "进入聊天", "進入聊天", "チャットへ", "Done", "完成",
            "Next", "下一步", "Skip", "跳过"
        ]
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            guard let button = waitAnyButton(buttons, timeout: 3) else { return }
            button.tap()
        }
    }

    private func waitAnyButton(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(min(timeout, 10))
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists, element.isEnabled, element.isHittable { return element }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }
}
