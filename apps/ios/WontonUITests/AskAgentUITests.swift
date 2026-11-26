// “问 Bot”的失败提示：一句按原因写的人话、重试，重试成功后按回执房间打开私聊。
import XCTest

final class AskAgentUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_FAKE_ASK"] = "contact_required_once"
        UITestBackend.launch(app)
    }

    func testAskFailureExplainsAndRetryOpensTheAskedRoom() {
        app.open(URL(string: "wonton:agent-chat")!)
        let bar = app.otherElements["askFailureBar"]
        XCTAssertTrue(bar.waitForExistence(timeout: 8), "没有失败提示\n\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["This Bot doesn't have you as a contact yet."].exists,
                      "提示不是按类别写的原因\n\(app.debugDescription)")
        shot("ask-failure")

        app.buttons["Try Again"].tap()
        XCTAssertTrue(app.textViews["Message"].waitForExistence(timeout: 8),
                      "重试成功后没有打开回执房间\n\(app.debugDescription)")
        shot("ask-retried")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
