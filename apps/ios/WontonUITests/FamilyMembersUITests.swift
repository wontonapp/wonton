import XCTest

/// 成员页实走（ADR-0088）：主人打开设置里的“成员”，邀请一个人，再进角色页建一个新角色。
/// 要开发栈（`UITestBackend.require`），没有就整类跳过。
final class FamilyMembersUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func passOnboarding() {
        for _ in 0..<60 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            if UITestSidebar.waitForMain(app, timeout: 1) { return }
            if let skip = UITestSidebar.button(["Skip", "跳过"], app: app), skip.isEnabled { skip.tap(); continue }
            if let next = UITestSidebar.button(["Next", "下一步"], app: app), next.isEnabled { next.tap(); continue }
            if let done = UITestSidebar.button(["Done", "完成"], app: app), done.isEnabled { done.tap(); continue }
        }
    }

    func testInviteAndRoles() throws {
        passOnboarding()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 90), "没进主界面")
        guard let settings = UITestSidebar.row(app, labels: ["Settings", "设置"]) else {
            return XCTFail("侧边栏没有设置")
        }
        settings.tap()
        guard let members = waiting(["Members"], timeout: 90) else {
            return XCTFail("设置里没有成员入口\n\(app.debugDescription)")
        }
        shot("members-entry")
        members.tap()

        guard let invite = waiting(["Invite"], timeout: 30) else { return XCTFail("没进成员页") }
        invite.tap()
        let name = app.textFields.firstMatch
        XCTAssertTrue(name.waitForExistence(timeout: 15), "没有名字输入框")
        name.tap()
        name.typeText("Ada")
        app.buttons["Create invite"].firstMatch.tap()
        XCTAssertTrue(app.images["Invite QR code"].waitForExistence(timeout: 60), "没出邀请码")
        shot("invite-qr")

        app.buttons["Done"].firstMatch.tap()
        guard let roles = waiting(["Roles"], timeout: 20) else { return XCTFail("没有角色入口") }
        roles.tap()
        guard let add = waiting(["New role"], timeout: 20) else { return XCTFail("角色页没打开") }
        add.tap()
        let roleName = app.textFields.firstMatch
        XCTAssertTrue(roleName.waitForExistence(timeout: 15), "没有角色名输入框")
        roleName.tap()
        roleName.typeText("Guest")
        app.buttons["Save role"].firstMatch.tap()
        XCTAssertTrue(waiting(["Guest"], timeout: 30) != nil, "角色没保存")
        shot("roles")
    }

    private func waiting(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
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
}
