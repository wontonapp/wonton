import XCTest

/// 成员、角色和加入页的截图夹具（ADR-0088）：核心假数据，不连服务器。
/// 参考图由 CI 的截图流程出，平时当界面冒烟用。
@MainActor
final class FamilyFakeScreensUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = "family-fake"
    }

    private func launch(_ variant: String, scheme: String) {
        app.launchEnvironment["WONTON_SHOT_SCHEME"] = scheme
        if variant.isEmpty {
            app.launchArguments.append("wonton-family-preview")
        } else {
            app.launchArguments.append("wonton-family-preview=\(variant)")
        }
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testMembersLight() {
        launch("", scheme: "light")
        XCTAssertTrue(app.buttons["Invite"].waitForExistence(timeout: 20), "成员页没出来\n\(app.debugDescription)")
        shot("members-light")
    }

    func testMembersDark() {
        launch("", scheme: "dark")
        XCTAssertTrue(app.buttons["Invite"].waitForExistence(timeout: 20), "成员页没出来")
        shot("members-dark")
    }

    func testRoles() {
        launch("roles", scheme: "light")
        guard let add = app.buttons["New role"].waitForExistence(timeout: 20) ? app.buttons["New role"] : nil else {
            return XCTFail("角色页没出来\n\(app.debugDescription)")
        }
        shot("roles")
        add.tap()
        let name = app.textFields["family-role-name"]
        XCTAssertTrue(name.waitForExistence(timeout: 10), "角色名输入框没出来")
        name.tap()
        name.typeText("Guest\n")
        shot("role-editor")
        app.buttons["Save role"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Guest"].waitForExistence(timeout: 15), "角色没保存")
        shot("role-saved")
    }

    func testJoin() {
        launch("join", scheme: "light")
        XCTAssertTrue(app.otherElements["family-join-page"].waitForExistence(timeout: 20), "加入页没出来\n\(app.debugDescription)")
        shot("join")
    }
}
