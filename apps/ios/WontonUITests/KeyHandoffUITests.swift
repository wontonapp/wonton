import XCTest

/// 加设备时交接聊天密钥（ADR-0087）实走：配对码由开着“添加设备”的旧设备给（`key-handoff-old-device` 示例扮演，
/// 它对表情时替主人点“相同”）。这台配上后整屏是“在另一台设备上继续”，不出“输入恢复密钥”；两边对一次表情，
/// 这台点“相同”后密钥过来，交接页自己收起。要旧设备在跑，所以只在给了 `TEST_RUNNER_KEY_HANDOFF` 时跑。
@MainActor
final class KeyHandoffUITests: XCTestCase {
    func testPairedPhoneGetsTheKeysFromTheOtherDevice() throws {
        continueAfterFailure = false
        try XCTSkipIf(ProcessInfo.processInfo.environment["KEY_HANDOFF"] == nil, "要旧设备开着“添加设备”")
        try UITestBackend.require()
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()

        // 配对、登录 Matrix、查对方设备都经开发栈。
        let title = app.staticTexts["Continue on your other device"]
        XCTAssertTrue(title.waitForExistence(timeout: 90))
        shot("handoff-page")
        let match = app.buttons["Match"]
        XCTAssertTrue(match.waitForExistence(timeout: 90))
        shot("handoff-compare")
        XCTAssertFalse(app.staticTexts["Enter your recovery key to restore past messages"].exists)
        match.tap()

        XCTAssertTrue(title.waitForNonExistence(timeout: 90))
        shot("handoff-finished")
        XCTAssertFalse(app.staticTexts["Enter your recovery key to restore past messages"].exists)
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
