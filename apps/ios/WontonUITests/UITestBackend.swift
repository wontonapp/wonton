import XCTest

/// 要后端（开发栈的核心和 Tuwunel）的走屏共用的前置：
/// 夜跑不连后端，没有配对码时整条用例 XCTSkip。配对码由 `deploy/dev/core/dev.sh pair <地址>` 打出，10 分钟内有效，
/// 经 `apps/ios/scripts/ui-test.sh`（xcodebuild 的 `TEST_RUNNER_PAIRING_CODE`）进测试进程，变量名去掉前缀后是
/// `PAIRING_CODE`；可以是逗号分隔的几个码（码里的逗号已按 URL 编码），排队久了前面的过期时按顺序换下一个。
enum UITestBackend {
    /// 给了码但不像配对码：多半是传的路上坏了（截断、和别的输出交错）。用例直接失败，不把坏码交给 App。
    struct MalformedPairingCode: Error, CustomStringConvertible {
        let index: Int
        let length: Int
        let head: String

        var description: String {
            "PAIRING_CODE 第 \(index + 1) 个不是配对码（长度 \(length)，开头 \(head.debugDescription)）：传码的路上坏了"
        }
    }

    static func codes() throws -> [String] {
        let raw = ProcessInfo.processInfo.environment["PAIRING_CODE"] ?? ""
        if raw.isEmpty { return [] }
        let codes = raw.split(separator: ",", omittingEmptySubsequences: false).map(String.init)
        for (index, code) in codes.enumerated() where !looksLikePairingCode(code) {
            // 只露出不含密钥的开头。
            throw MalformedPairingCode(index: index, length: code.count, head: String(code.prefix(14)))
        }
        return codes
    }

    static func require() throws {
        let missing = try codes().isEmpty
        try XCTSkipIf(missing, "需要开发栈：没有 PAIRING_CODE")
    }
    static func enableStateProbe(to app: XCUIApplication) {
        app.launchEnvironment["WONTON_UI_TEST_STATE"] = "1"
    }
    /// 每次启动都用一个空的数据目录，再带上第一个配对码：不依赖模拟器上留下的会话。
    static func passPairingCode(to app: XCUIApplication) throws {
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        enableStateProbe(to: app)
        if let code = try codes().first {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
    }

    /// 只处理已经出现的交接入口；引导循环和收尾等待共用同一条导航路径。
    @discardableResult
    static func skipKeyHandoff(_ app: XCUIApplication) -> Bool {
        let skip = app.buttons["key-handoff-skip"]
        if skip.exists && skip.isHittable {
            skip.tap()
            let backup = app.scrollViews["chat-backup"]
            XCTAssertTrue(backup.waitForExistence(timeout: 10), app.debugDescription)
            return closeChatBackup(app)
        }
        if app.scrollViews["chat-backup"].exists { return closeChatBackup(app) }
        let reminder = app.buttons["chat-backup-reminder-dismiss"]
        if reminder.exists && reminder.isHittable {
            reminder.tap()
            XCTAssertTrue(reminder.waitForNonExistence(timeout: 10), app.debugDescription)
            return true
        }
        return false
    }

    /// 等核心结束 Checking，而不是等一个不一定出现的按钮或猜测一段安静时间。
    static func settleKeyHandoff(_ app: XCUIApplication, timeout: TimeInterval = 90) {
        let state = app.descendants(matching: .any)["key-handoff-test-state"].firstMatch
        let resolved = NSPredicate { _, _ in
            app.buttons["key-handoff-skip"].isHittable
                || app.scrollViews["chat-backup"].exists
                || (state.exists && state.value as? String == "settled")
        }
        let result = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: resolved, object: app)], timeout: timeout
        )
        guard result == .completed else {
            XCTFail("交接判定未完成：\(app.debugDescription)")
            return
        }
        _ = skipKeyHandoff(app)
        let finished = NSPredicate { _, _ in
            state.exists && state.value as? String == "settled"
                && !app.buttons["key-handoff-skip"].exists
                && !app.scrollViews["chat-backup"].exists
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: finished, object: app)], timeout: 10),
            .completed, app.debugDescription
        )
        _ = skipKeyHandoff(app)
    }

    private static func closeChatBackup(_ app: XCUIApplication) -> Bool {
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回", "戻る"])).firstMatch
        let ready = NSPredicate { _, _ in back.exists && back.isHittable }
        guard XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: ready, object: app)], timeout: 10)
            == .completed else {
            XCTFail("聊天备份页没有可用的返回入口：\(app.debugDescription)")
            return false
        }
        back.tap()
        XCTAssertTrue(app.scrollViews["chat-backup"].waitForNonExistence(timeout: 10), app.debugDescription)
        return true
    }

    /// 和核心 `PairingCode::parse` 的第一道检查一致：`wonton://pair?` 开头的一整行，中间没有空白。
    private static func looksLikePairingCode(_ code: String) -> Bool {
        code.hasPrefix("wonton://pair?") && code.count > 14
            && !code.unicodeScalars.contains { CharacterSet.whitespacesAndNewlines.contains($0) }
    }
}
