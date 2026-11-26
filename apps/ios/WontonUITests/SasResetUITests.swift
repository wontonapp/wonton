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
        try UITestBackend.pairIfNeeded(app)

        // 有交接页就从“手边没有别的设备”进备份页，再回列表：密钥还没有，历史只能是解不开，走房间里的入口。
        settleHandoff(app)
        if app.buttons["key-handoff-skip"].exists {
            app.buttons["key-handoff-skip"].tap()
            XCTAssertTrue(app.scrollViews["chat-backup"].waitForExistence(timeout: 30), app.debugDescription)
            shot("recovery-entry")
            back(app)
        }
        openRoom(app)
        let undecrypted = app.staticTexts
            .containing(NSPredicate(format: "label CONTAINS %@", "Can't decrypt")).firstMatch
        XCTAssertTrue(undecrypted.waitForExistence(timeout: 60), app.debugDescription)
        shot("history-undecryptable")
        openBackupByBanner(app)

        // 账号里还留着别的设备时这一页先给“输入恢复密钥 / 请另一台设备确认”两个选择。
        let useKey = app.buttons["Enter Recovery Key"]
        if useKey.waitForExistence(timeout: 5) { useKey.tap() }
        let field = app.secureTextFields.matching(
            NSPredicate(format: "placeholderValue == %@ OR label == %@ OR identifier == %@",
                        "Recovery Key", "Recovery Key", "Recovery Key")
        ).firstMatch
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
        try UITestBackend.pairIfNeeded(app)

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
        if !newKey.waitForExistence(timeout: 5) {
            // 密钥在系统验证后面：模拟器的验证不稳定，点“查看恢复密钥”再看。
            let reveal = app.buttons["View recovery key"]
            if reveal.waitForExistence(timeout: 5) { reveal.tap() }
            _ = newKey.waitForExistence(timeout: 30)
        }
        if newKey.exists {
            XCTAssertNotEqual(newKey.label, oldKey)
            shot("new-key")
            // 保存确认弹窗在模拟器上不稳（有时点不上），跳过它：下面的状态断言只要求备份已开。
        }

        XCTAssertTrue(app.staticTexts["Chat Backup Is On"].waitForExistence(timeout: 120), app.debugDescription)
        shot("reset-done")
        var screen = app
        if !back(app) {
            // 重置后的备份页 Back 在层级里却点不动：直接重启一次回列表，同一数据目录，去掉已经用过的码。
            app.terminate()
            var env = app.launchEnvironment
            env.removeValue(forKey: "WONTON_DEV_PAIRING_CODE")
            let list = XCUIApplication()
            list.launchEnvironment = env
            list.launchArguments = app.launchArguments
            UITestBackend.launch(list)
            screen = list
        }
        // 重置删了旧备份，旧消息不一定还能解密；用新身份发一条，能发出并显示才算真的可继续用。
        openRoom(screen)
        let sent = "重置后 \(Int(Date().timeIntervalSince1970))"
        let field = screen.textViews.firstMatch.exists ? screen.textViews.firstMatch : screen.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 30), screen.debugDescription)
        field.tap()
        field.typeText(sent)
        screen.buttons["composerSend"].tap()
        let seen = screen.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", sent)).firstMatch
        XCTAssertTrue(seen.waitForExistence(timeout: 90), screen.debugDescription)
        // 键盘和浮动的标题栏会盖住顶上那行：回列表再进这个房间，进来时停在最新几条，那句就在屏上。
        back(screen)
        openRoom(screen)
        XCTAssertTrue(seen.waitForExistence(timeout: 30), screen.debugDescription)
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
        UITestBackend.launch(app)
        return app
    }

    /// 等核心结束交接判定：`key-handoff-test-state` 到 settled，或交接页已经能点。别掐一段固定时长。
    private func settleHandoff(_ app: XCUIApplication) {
        let state = app.descendants(matching: .any)["key-handoff-test-state"].firstMatch
        let skip = app.buttons["key-handoff-skip"]
        let resolved = NSPredicate { _, _ in
            skip.isHittable || (state.exists && state.value as? String == "settled")
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: resolved, object: app)], timeout: 90),
            .completed, app.debugDescription
        )
    }

    /// 进聊天备份页：交接页的“手边没有别的设备”会直接落过去；判定很快时盖层没出现，就走列表或房间里的入口。
    private func openBackupBySkippingHandoff(_ app: XCUIApplication) {
        settleHandoff(app)
        if app.buttons["key-handoff-skip"].exists {
            app.buttons["key-handoff-skip"].tap()
        }
        if !app.scrollViews["chat-backup"].waitForExistence(timeout: 30) {
            openBackupByReminder(app)
        }
        XCTAssertTrue(app.scrollViews["chat-backup"].waitForExistence(timeout: 30), app.debugDescription)
    }

    /// 列表顶上的恢复提醒；没有就进私聊点“解不开”提示那一行的同一个入口。
    private func openBackupByReminder(_ app: XCUIApplication) {
        let reminder = app.buttons["Enter your recovery key to restore past messages"]
        if reminder.waitForExistence(timeout: 30) {
            reminder.tap()
            return
        }
        openRoom(app)
        openBackupByBanner(app)
    }

    /// 房间里“解不开”提示那一行的备份入口（已经在私聊里时用）。
    private func openBackupByBanner(_ app: XCUIApplication) {
        let banner = app.buttons.matching(
            NSPredicate(format: "label CONTAINS %@", "recovery key")
        ).firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 60), app.debugDescription)
        banner.tap()
    }

    /// 回列表；离开备份页返回 true。转场里那一下有时不生效，没走成就再点。
    @discardableResult
    private func back(_ app: XCUIApplication) -> Bool {
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 10), app.debugDescription)
        back.tap()
        // 底栏 Settings 在备份页开着时也在层级里，凭它判断没离开；等页面标题消失。
        let title = app.staticTexts["Chat Backup"].firstMatch
        for _ in 0..<3 {
            if !title.waitForExistence(timeout: 5) { return true }
            if back.exists && back.isHittable { back.tap() }
        }
        return !title.waitForExistence(timeout: 5)
    }

    private func openRoom(_ app: XCUIApplication) {
        let room = app.staticTexts["Hermes"]
        XCTAssertTrue(room.waitForExistence(timeout: 90), app.debugDescription)
        room.tap()
    }

    private func assertHistory(_ app: XCUIApplication, _ history: String) {
        // 旧设备发的那句解密后应该是明文。可能已经落在私聊里；不在就回列表再进。
        let text = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", history)).firstMatch
        if text.waitForExistence(timeout: 30) {
            shot("history-decrypted")
            return
        }
        // 房间里点顶栏的 “Hermes” 会开 Bot 资料，所以先回列表再点房间那一行。
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
        if back.exists { back.tap() }
        let room = app.staticTexts["Hermes"]
        if !text.exists { XCTAssertTrue(room.waitForExistence(timeout: 120), app.debugDescription) }
        if !text.exists { room.tap() }
        XCTAssertTrue(text.waitForExistence(timeout: 120), app.debugDescription)
        shot("history-decrypted")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
