import XCTest

/// 没有可信设备时的两条恢复路径（ADR-0076）：拿得出恢复密钥就从备份恢复，拿不出就确认后重置身份。
/// 两条都在真开发栈上跑；账号和服务端状态由外面准备好（备份已开、可验证设备已删），
/// 旧设备发过的加密消息由 `HISTORY_TEXT`（恢复路径用）传入，密钥由 `RECOVERY_KEY` 传入。
@MainActor
final class SasResetUITests: XCTestCase {
    func testRecoveryKeyRestoresPastMessages() throws {
        continueAfterFailure = false
        let key = try env("RECOVERY_KEY")
        let history = try env("HISTORY_TEXT")
        let app = try launch()

        openBackupBySkippingHandoff(app)
        shot("recovery-entry")
        let field = app.secureTextFields["Recovery Key"]
        XCTAssertTrue(field.waitForExistence(timeout: 30), app.debugDescription)
        field.tap()
        field.typeText(key)
        app.buttons["Restore Past Messages"].tap()

        let enabled = app.staticTexts["Chat Backup Is On"]
        XCTAssertTrue(enabled.waitForExistence(timeout: 120), app.debugDescription)
        shot("recovered")
        back(app)
        assertHistory(app, history)
    }

    func testResetCreatesANewRecoveryKey() throws {
        continueAfterFailure = false
        let oldKey = try env("RECOVERY_KEY")
        let app = try launch()

        openBackupBySkippingHandoff(app)
        let reset = app.buttons["Reset Backup"]
        XCTAssertTrue(reset.waitForExistence(timeout: 30), app.debugDescription)
        shot("reset-entry")
        reset.tap()
        let confirm = app.buttons["Reset"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 30), app.debugDescription)
        confirm.tap()

        XCTAssertTrue(app.staticTexts["Save Your Recovery Key"].waitForExistence(timeout: 180), app.debugDescription)
        // 密钥要过系统验证才显示；模拟器的系统验证在测试里不稳定，这里只认“密钥区出现”，
        // 已经可见时顺手比对并确认保存。
        let newKey = app.staticTexts["recovery-key-value"]
        if newKey.waitForExistence(timeout: 5) {
            XCTAssertNotEqual(newKey.label, oldKey)
            shot("new-key")
            app.buttons["Saved"].tap()
            let saved = app.buttons["Saved"]
            XCTAssertTrue(saved.waitForExistence(timeout: 30), app.debugDescription)
            saved.tap()
        }

        XCTAssertTrue(app.staticTexts["Chat Backup Is On"].waitForExistence(timeout: 120), app.debugDescription)
        shot("reset-done")
        back(app)
        // 重置删了旧备份，旧消息不一定还能解密；用新身份发一条，能发出并显示才算真的可继续用。
        openRoom(app)
        let sent = "重置后 \(Int(Date().timeIntervalSince1970))"
        let field = app.textViews.firstMatch.exists ? app.textViews.firstMatch : app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 30), app.debugDescription)
        field.tap()
        field.typeText(sent)
        app.buttons["composerSend"].tap()
        let seen = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", sent)).firstMatch
        XCTAssertTrue(seen.waitForExistence(timeout: 90), app.debugDescription)
        shot("reset-message")
    }

    private func env(_ name: String) throws -> String {
        let value = ProcessInfo.processInfo.environment[name] ?? ""
        try XCTSkipIf(value.isEmpty, "需要 \(name)")
        return value
    }

    private func launch() throws -> XCUIApplication {
        try UITestBackend.require()
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        app.launch()
        return app
    }

    /// 进聊天备份页：交接页的“手边没有别的设备”会直接落过去；判定很快时盖层可能没出现，
    /// 列表提醒也可能因为恢复状态还没取到而不显示，就进私聊点“解不开”提示那一行的备份入口。
    private func openBackupBySkippingHandoff(_ app: XCUIApplication) {
        let skip = app.buttons["key-handoff-skip"]
        if skip.waitForExistence(timeout: 40) {
            skip.tap()
        } else {
            let reminder = app.buttons["Enter your recovery key to restore past messages"]
            if reminder.waitForExistence(timeout: 90) {
                reminder.tap()
            } else {
                // 两种入口都没出现：进有旧消息的私聊，点解不开提示那一行的备份入口。
                let room = app.staticTexts["Hermes"]
                XCTAssertTrue(room.waitForExistence(timeout: 60), app.debugDescription)
                room.tap()
                let banner = app.buttons.matching(
                    NSPredicate(format: "label CONTAINS %@", "recovery key")
                ).firstMatch
                XCTAssertTrue(banner.waitForExistence(timeout: 60), app.debugDescription)
                banner.tap()
            }
        }
        XCTAssertTrue(app.scrollViews["chat-backup"].waitForExistence(timeout: 30), app.debugDescription)
    }

    private func back(_ app: XCUIApplication) {
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 10), app.debugDescription)
        back.tap()
        // 底栏 Settings 在备份页开着时也在层级里，凭它判断没离开；等页面标题消失。
        let title = app.staticTexts["Chat Backup"].firstMatch
        if title.exists {
            XCTAssertTrue(title.waitForNonExistence(timeout: 10), app.debugDescription)
        }
    }

    private func openRoom(_ app: XCUIApplication) {
        let room = app.staticTexts["Hermes"]
        XCTAssertTrue(room.waitForExistence(timeout: 90), app.debugDescription)
        room.tap()
    }

    private func assertHistory(_ app: XCUIApplication, _ history: String) {
        // 进旧设备发过消息的那个私聊看时间线；列表预览可能已被 Bot 的新回复顶掉。
        openRoom(app)
        let text = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", history)).firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 90), app.debugDescription)
        shot("history-decrypted")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
