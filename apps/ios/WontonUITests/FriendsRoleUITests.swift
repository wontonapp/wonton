import XCTest

/// 好友页实走（ADR-0051）：按 `TEST_RUNNER_ROLE` 分两端。
/// A：打开好友页、发邀请、同意、屏蔽邀请、拉黑服务器、撤销、删好友。
/// B：空列表、收到请求后拒绝、再收到后同意并看到好友行。
final class FriendsRoleUITests: XCTestCase {
    private var app: XCUIApplication!
    private var role = ""

    override func setUpWithError() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        role = env["TEST_RUNNER_ROLE"] ?? ""
        try XCTSkipIf(role.isEmpty, "没给 TEST_RUNNER_ROLE")
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "\(role)-\(name)"
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

    private func waitText(_ text: String, timeout: TimeInterval = 120) -> XCUIElement? {
        let element = app.staticTexts.matching(NSPredicate(format: "label == %@", text)).firstMatch
        return element.waitForExistence(timeout: timeout) ? element : nil
    }

    private func tabButton(_ labels: [String]) -> XCUIElement? {
        let height = app.windows.firstMatch.frame.height
        for label in labels {
            for element in app.buttons.matching(identifier: label).allElementsBoundByIndex
            where element.frame.minY > height * 0.7 { return element }
        }
        return nil
    }

    private func passOnboarding() {
        for _ in 0..<60 {
            if tabButton(["Chats", "Apps"]) != nil { return }
            if let skip = waitAnyButton(["Skip"], timeout: 2), skip.isEnabled { skip.tap(); continue }
            if let done = waitAnyButton(["Done"], timeout: 2), done.isEnabled { done.tap(); continue }
            if let next = waitAnyButton(["Next"], timeout: 2), next.isEnabled { next.tap(); continue }
        }
    }

    /// 设置 → 好友；入口可能要等列表取回来。
    private func openFriends() {
        guard let settings = tabButton(["Settings"]) else { return XCTFail("没有 Settings") }
        settings.tap()
        guard let friends = waitAnyButton(["Friends"], timeout: 90) else { return XCTFail("设置里没有好友入口") }
        friends.tap()
        XCTAssertTrue(waitAnyButton(["Add"], timeout: 30) != nil, "好友页没打开")
    }

    private func addFriend(_ userId: String) {
        XCTAssertTrue(waitAnyButton(["Add"], timeout: 30) != nil, "没有加好友按钮")
        waitAnyButton(["Add"])?.tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 15), "没有输入框")
        field.tap()
        field.typeText(userId)
        app.buttons["Send request"].firstMatch.tap()
        XCTAssertTrue(waitText("Waiting for them to accept", timeout: 30) != nil, "邀请没发出去")
        shot("01-invite-sent")
    }

    private func back() {
        if let back = waitAnyButton(["Back"], timeout: 5) { back.tap() }
    }

    func testRole() throws {
        _ = waitAnyButton(["Chats", "Apps", "Skip"], timeout: 90)
        passOnboarding()
        XCTAssertNotNil(tabButton(["Chats"]), "没进主界面")
        tabButton(["Chats"])?.tap()
        openFriends()
        shot("00-list")

        if role == "a" {
            try runInviter()
        } else {
            try runInvitee()
        }
    }

    private func runInviter() throws {
        let peer = ProcessInfo.processInfo.environment["TEST_RUNNER_PEER"] ?? ""
        XCTAssertFalse(peer.isEmpty, "没给 TEST_RUNNER_PEER")
        addFriend(peer)

        guard let accept = waitAnyButton(["Approve", "Accept"], timeout: 180) else {
            return XCTFail("没等到对方的请求")
        }
        shot("02-request")
        accept.tap()
        XCTAssertTrue(waitAnyButton(["Block invitations"], timeout: 60) != nil, "同意后没有好友行")
        shot("03-friend")

        let peerId = ProcessInfo.processInfo.environment["TEST_RUNNER_PEER"] ?? ""
        waitAnyButton(["Block invitations"])?.tap()
        XCTAssertTrue(waitText(peerId, timeout: 30) != nil, "屏蔽区没有这个人")
        shot("04-blocked")
        waitAnyButton(["Undo"])?.tap()
        XCTAssertTrue(waitText(peerId, timeout: 20) == nil, "撤销后还在屏蔽区")
        shot("05-unblocked")

        waitAnyButton(["Block Server"])?.tap()
        let field = app.alerts.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 15), "没有服务器输入框")
        let server = ProcessInfo.processInfo.environment["TEST_RUNNER_SERVER"] ?? ""
        field.tap()
        field.typeText(server)
        app.alerts.buttons["Block Server"].firstMatch.tap()
        XCTAssertTrue(waitText(server, timeout: 30) != nil, "服务器没拉黑")
        shot("06-server-blocked")
        waitAnyButton(["Undo"])?.tap()
        XCTAssertTrue(waitText(server, timeout: 20) == nil, "服务器屏蔽没撤销")
        shot("07-server-unblocked")

        let row = app.staticTexts["Owner"].firstMatch.exists
            ? app.staticTexts["Owner"].firstMatch
            : app.staticTexts.matching(NSPredicate(format: "label CONTAINS '@'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 30), "没有好友行可删")
        row.swipeLeft()
        waitAnyButton(["Remove Friend"], timeout: 15)?.tap()
        if waitAnyButton(["Remove Friend"], timeout: 5) != nil {
            app.buttons["Remove Friend"].firstMatch.tap()
        }
        XCTAssertTrue(waitText("No friends yet", timeout: 60) != nil, "删了还在列表")
        shot("08-removed")
    }

    private func runInvitee() throws {
        XCTAssertTrue(waitText("No friends yet", timeout: 60) != nil, "空列表没显示")
        shot("10-empty")

        guard let decline = waitAnyButton(["Decline"], timeout: 180) else {
            return XCTFail("没等到请求")
        }
        shot("11-request")
        decline.tap()
        XCTAssertTrue(waitText("No friends yet", timeout: 60) != nil, "拒绝后请求还在")

        XCTAssertTrue(waitAnyButton(["Approve", "Accept"], timeout: 180) != nil, "没等到第二次请求")
        shot("12-request-again")
        waitAnyButton(["Approve", "Accept"])?.tap()
        XCTAssertTrue(waitAnyButton(["Block invitations"], timeout: 60) != nil, "同意后没有好友行")
        shot("13-friend")
    }
}
