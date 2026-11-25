import XCTest

/// 加设备时交接聊天密钥（ADR-0087）实走：配对码由开着“添加设备”的旧设备给（`key-handoff-old-device` 示例扮演，
/// 它对表情时替主人点“相同”）。这台配上后整屏是“在另一台设备上继续”，不出“输入恢复密钥”；两边对一次表情，
/// 这台点“相同”后密钥过来，交接页自己收起。要旧设备在跑，所以只在给了 `TEST_RUNNER_KEY_HANDOFF` 时跑。
/// 旧设备把表情按序打到 stdout，外面收 `表情：` 那行写进本 App 的 Documents（外面读不到粘贴板，见 README），
/// 这里对着屏幕比。
@MainActor
final class KeyHandoffUITests: XCTestCase {
    func testPairedPhoneGetsTheKeysFromTheOtherDevice() throws {
        continueAfterFailure = false
        try XCTSkipIf(ProcessInfo.processInfo.environment["KEY_HANDOFF"] == nil, "要旧设备开着“添加设备”")
        try UITestBackend.require()
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
        try UITestBackend.pairIfNeeded(app)

        // 配对、登录 Matrix、查对方设备都经开发栈。
        let title = app.staticTexts["Continue on your other device"]
        XCTAssertTrue(title.waitForExistence(timeout: 90))
        shot("handoff-page")
        let match = app.buttons["Match"]
        XCTAssertTrue(match.waitForExistence(timeout: 90))
        shot("handoff-compare")
        XCTAssertFalse(app.staticTexts["Enter your recovery key to restore past messages"].exists)
        assertSameEmoji(app)
        match.tap()

        XCTAssertTrue(title.waitForNonExistence(timeout: 90))
        shot("handoff-finished")
        XCTAssertFalse(app.staticTexts["Enter your recovery key to restore past messages"].exists)

        // 旧设备在交接前发过一句加密消息：密钥到齐后时间线里应该是明文。
        guard let history = ProcessInfo.processInfo.environment["HISTORY_TEXT"], !history.isEmpty else {
            print("HISTORY_TEXT 没设：明文断言不跑；旧设备要同一个 KEY_HANDOFF_HISTORY_TEXT")
            return
        }
        assertHistory(app, history)
    }

    /// 两端必须显示同一组表情、同一顺序：对不上就是两台换了不同的东西，主人却点了“相同”。
    /// 一次取全部再按纵坐标排：逐个 `staticTexts[symbol]` 取时，重复的符号两次落到同一节点，顺序错也能过。
    private func assertSameEmoji(_ app: XCUIApplication) {
        let expected = waitForEmoji()
        // 表情在文件写好前就可能已经上屏，也可能晚一拍：等到够数再取，别把还没画出来的当成没有。
        var rows: [(CGFloat, String)] = []
        let deadline = Date().addingTimeInterval(10)
        repeat {
            rows = app.staticTexts.allElementsBoundByIndex
                .filter { expected.contains($0.label) }
                .map { ($0.frame.midY, $0.label) }
            if rows.count >= expected.count { break }
            Thread.sleep(forTimeInterval: 0.5)
        } while Date() < deadline
        XCTAssertEqual(rows.sorted { $0.0 < $1.0 }.map(\.1), expected, "两端表情不同：旧设备 \(expected)")
    }

    /// 旧设备的 `表情：` 行由外面写进本 App 的 Documents/sas-emoji.txt；符号之间一个空格。
    private func waitForEmoji() -> [String] {
        let deadline = Date().addingTimeInterval(120)
        while Date() < deadline {
            if let dir = appDocuments(),
               let raw = try? String(contentsOf: dir.appendingPathComponent("sas-emoji.txt"), encoding: .utf8) {
                let symbols = raw.split(separator: " ").map(String.init)
                if symbols.count == 7 { return symbols }
            }
            Thread.sleep(forTimeInterval: 1)
        }
        XCTFail("没收到旧设备的表情：App 的 Documents/sas-emoji.txt 没有 7 个符号")
        return []
    }

    /// 被测 App 的 Documents：测试进程和 App 的数据容器是兄弟目录，按容器元数据里的 bundle ID 找。
    private func appDocuments() -> URL? {
        let applications = URL(fileURLWithPath: NSHomeDirectory()).deletingLastPathComponent()
        let siblings = (try? FileManager.default.contentsOfDirectory(
            at: applications, includingPropertiesForKeys: nil
        )) ?? []
        for dir in siblings {
            let metadata = dir.appendingPathComponent(".com.apple.mobile_container_manager.metadata.plist")
            guard let data = try? Data(contentsOf: metadata),
                  let plist = try? PropertyListSerialization.propertyList(from: data, format: nil),
                  let fields = plist as? [String: Any],
                  fields["MCMMetadataIdentifier"] as? String == "app.wonton.ios"
            else { continue }
            return dir.appendingPathComponent("Documents")
        }
        return nil
    }

    /// 旧设备发的那句解密后应该是明文。交接完成后可能已经落在私聊里；不在就回列表再进。
    private func assertHistory(_ app: XCUIApplication, _ history: String) {
        let text = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", history)).firstMatch
        if text.waitForExistence(timeout: 30) {
            shot("history-decrypted")
            return
        }
        // 房间里点顶栏的 “Hermes” 会开 Bot 资料，所以先回列表再点房间那一行。
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
        if back.exists { back.tap() }
        let room = app.staticTexts["Hermes"]
        XCTAssertTrue(room.waitForExistence(timeout: 180), app.debugDescription)
        // 回列表时可能还在换场、点不进去：等那行能点，再点、等，没进去就再来一轮。
        let open = NSPredicate { _, _ in room.exists && room.isHittable }
        _ = XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: open, object: app)], timeout: 60)
        for _ in 0..<3 {
            if text.waitForExistence(timeout: 60) { break }
            if back.exists { back.tap() }
            if room.exists && room.isHittable { room.tap() }
        }
        XCTAssertTrue(text.waitForExistence(timeout: 60), app.debugDescription)
        shot("history-decrypted")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
