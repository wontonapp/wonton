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
    /// 配对码只能用一次，每次启动都换一个：一个类里每条用例、一个用例里浅色/深色两次启动都是新数据目录（等于新设备），
    /// 各占一个码。启动时带的是第 `pairingCursor` 个，配对失败（已用、过期）就换下一个。
    nonisolated(unsafe) private static var pairingCursor = 0

    /// 每次启动都用一个空的数据目录，再带上下一个没用过的配对码：不依赖模拟器上留下的会话。
    /// 通知权限也统一交给用例自己问：启动自动弹框会吃掉各用例的第一个点击。
    static func passPairingCode(to app: XCUIApplication) throws {
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_DEV_SKIP_PUSH_AUTH"] = "1"
        enableStateProbe(to: app)
        let codes = try codes()
        if !codes.isEmpty {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = codes[min(pairingCursor, codes.count - 1)]
            pairingCursor += 1
        }
    }

    /// 启动 App。通知权限框默认在这里关掉：它在启动后弹，会吃掉各用例的第一个点击；
    /// 要用框的用例传 `skipPushAuth: false`（推送探针要等权限框）。
    static func launch(_ app: XCUIApplication, skipPushAuth: Bool = true) {
        if skipPushAuth {
            app.launchEnvironment["WONTON_DEV_SKIP_PUSH_AUTH"] = "1"
        }
        app.launch()
    }

    /// 配到离开配对页为止。启动时带的码可能已经被上一条用了：在失败面点“再扫一次”换下一个码；
    /// 已经配上就什么都不做。返回时配对页已经不在，后续断言各自等自己那一屏。
    @MainActor
    static func pairIfNeeded(_ app: XCUIApplication, onAttempt: ((Int) -> Void)? = nil) throws {
        let codes = try codes()
        try XCTSkipIf(codes.isEmpty, "需要开发栈：没有 PAIRING_CODE")
        let field = app.descendants(matching: .any)["pairing-code"].firstMatch
        // 失败面的判据用 scan-again：`pairing-failure` 标识运行时不进 a11y 树（见 `PairingFailureUITests`），
        // scan-again 是失败面独有的按钮。
        let again = app.buttons["pairing-scan-again"].firstMatch
        let progress = app.descendants(matching: .any)["pairing-progress"].firstMatch
        // 交接页、引导页出现就是配上了；主界面的侧栏在配对页后面也有，得等配对页真正收起。
        func paired() -> Bool {
            if app.staticTexts["Continue on your other device"].exists { return true }
            if app.buttons["key-handoff-skip"].exists { return true }
            if app.buttons["Skip"].exists { return true }
            if app.buttons["Apps"].exists || app.buttons["应用"].exists { return true }
            let pairing = field.exists || again.exists || progress.exists
            return !pairing && app.buttons["Sidebar"].exists
        }
        let start = min(pairingCursor, codes.count)
        let candidates = Array(codes[start...]) + codes[..<start]
        for (index, code) in candidates.enumerated() {
            if field.waitForExistence(timeout: 20) {
                field.tap()
                UIPasteboard.general.string = code
                field.press(forDuration: 1.2)
                let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"] : app.menuItems["粘贴"]
                if paste.exists {
                    paste.tap()
                } else {
                    field.typeText(code)
                }
                onAttempt?(index)
                let pair = app.buttons["pairing-submit"]
                XCTAssertTrue(pair.waitForExistence(timeout: 10), app.debugDescription)
                pair.tap()
            }
            let deadline = Date().addingTimeInterval(120)
            while Date() < deadline {
                if paired() {
                    pairingCursor = (start + index + 1) % codes.count
                    return
                }
                if again.exists { break }
                Thread.sleep(forTimeInterval: 1)
            }
            // 已用、过期的码在失败面换下一个；连失败面都没有就再等等，交给最后一条断言。
            if again.exists && again.isHittable { again.tap() }
        }
        XCTFail("配对码都不好使（已用或过期）：\(app.debugDescription)")
    }

    /// 只处理已经出现的交接入口；引导循环和收尾等待共用同一条导航路径。
    /// 引导期间聊天栈没挂载（OnboardingGate 只渲染引导页），配对没走完时备份页也来不及出现：
    /// 点了 Skip 等不到备份页就算了，硬断言只留在 settleKeyHandoff 的 finished 判定里。
    @discardableResult
    static func skipKeyHandoff(_ app: XCUIApplication) -> Bool {
        let skip = app.buttons["key-handoff-skip"]
        if skip.exists && skip.isHittable {
            skip.tap()
            if app.scrollViews["chat-backup"].waitForExistence(timeout: 10) { return closeChatBackup(app) }
            return true
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

    /// 下一个还没用过的码（用完最后一个就复用）；没有码时为 nil。
    static func nextCodeValue() -> String? {
        guard let all = try? codes(), !all.isEmpty else { return nil }
        let value = all[min(pairingCursor, all.count - 1)]
        pairingCursor += 1
        return value
    }

    /// 配对握手断了（隧道抖动）时 App 会停在「这个配对码已经用过了」的页面：
    /// 同一串码重试没用，换下一个码重新提交，最多两次。
    static func recoverPairing(_ app: XCUIApplication) {
        for _ in 0..<2 {
            // 独立失败页（pairing-refusal）和失败进度页（pairing-progress）都有同一个「Scan again」。
            let again = app.buttons["pairing-scan-again"]
            let end = Date().addingTimeInterval(2)
            while Date() < end, !again.exists { usleep(200_000) }
            guard again.exists, again.isHittable else { return }
            again.tap()
            guard let field = textField(app, "pairing-code", timeout: 5), let code = nextCodeValue() else { return }
            field.tap()
            field.typeText(code)
            let submit = app.buttons["pairing-submit"]
            if !submit.isHittable { app.swipeUp() }
            guard submit.waitForExistence(timeout: 5) else { return }
            submit.tap()
        }
    }

    private static func textField(_ app: XCUIApplication, _ id: String, timeout: TimeInterval) -> XCUIElement? {
        let field = app.textFields[id]
        return field.waitForExistence(timeout: timeout) ? field : nil
    }

    /// 等核心结束 Checking，而不是等一个不一定出现的按钮或猜测一段安静时间。
    static func settleKeyHandoff(_ app: XCUIApplication, timeout: TimeInterval = 90) {
        let state = app.descendants(matching: .any)["key-handoff-test-state"].firstMatch
        func entered() -> Bool {
            app.buttons["key-handoff-skip"].isHittable
                || app.scrollViews["chat-backup"].exists
                || (state.exists && state.value as? String == "settled")
        }
        // 配对失败页可能在等待中途才出现，所以边等边看，出现了就换码重来。
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline, !entered() {
            if app.descendants(matching: .any)["pairing-refusal"].exists
                || app.buttons["pairing-scan-again"].exists {
                recoverPairing(app)
            }
            usleep(300_000)
        }
        guard entered() else {
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
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: finished, object: app)], timeout: 30),
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
