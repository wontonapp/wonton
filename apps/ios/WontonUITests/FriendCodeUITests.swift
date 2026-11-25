import XCTest

/// 加好友的“我的二维码”和“扫对方的码”（ADR-0051）。要连开发栈（见 `UITestBackend`）。
///
/// - `testMyCodeShowsQR`：加好友面板 →“我的二维码”，看到码和自己的 matrix.to 链接。
/// - `testPastedLinkSendsRequest`：加好友面板 →“扫对方的码”，模拟器不能扫码，粘贴对方的 matrix.to 链接后发出请求。
///   对方的用户 ID 经 `TEST_RUNNER_PEER` 传入（同一台 Tuwunel 上的另一个账号，比如开发栈里 Bot 的账号）。
final class FriendCodeUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
        UITestBackend.settleKeyHandoff(app)
    }

    func testMyCodeShowsQR() throws {
        openAddFriend()
        tapButton("My QR Code", timeout: 40)
        XCTAssertTrue(app.images["My QR Code"].waitForExistence(timeout: 10), "没有二维码")
        let link = app.staticTexts["friends-my-link"]
        XCTAssertTrue(link.waitForExistence(timeout: 10), "没有链接")
        XCTAssertTrue(link.label.hasPrefix("https://matrix.to/#/@"), "链接不是 matrix.to 用户链接：\(link.label)")
        shot("my-code")
    }

    /// “我的二维码”的系统分享：出保存/复制图片项，保存到相册不崩、能回面板、能取消。
    /// 期望链接落 runner 沙盒固定文件，宿主机解码保存图后比对。
    /// 系统项/取消/权限框按设备语言配中英标签，权限框只点一次 Allow/允许。
    func testShareMyCodeQR() throws {
        openAddFriend()
        tapButton("My QR Code", timeout: 40)
        let link = app.staticTexts["friends-my-link"]
        XCTAssertTrue(link.waitForExistence(timeout: 10), "QR_SHARE_LINK_MISSING")
        let expectedLink = link.label
        // 落盘前只做布尔断言，不把载荷放进失败消息。
        XCTAssertFalse(expectedLink.isEmpty, "QR_EXPECTED_LINK_EMPTY")
        XCTAssertTrue(expectedLink.hasPrefix("https://matrix.to/#/@"), "QR_EXPECTED_LINK_PREFIX")
        // 固定文件名、无结尾换行；内容只进 runner 沙盒和宿主私有管道，不进附件、命令参数或日志。
        let expectedURL = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("qr-expected-link.txt")
        XCTAssertTrue(
            FileManager.default.createFile(atPath: expectedURL.path, contents: Data(expectedLink.utf8)),
            "QR_EXPECTED_WRITE_FAILED"
        )
        XCTAssertTrue(app.images["My QR Code"].exists, "没有二维码")
        let share = app.buttons["friends-my-share"]
        XCTAssertTrue(share.waitForExistence(timeout: 10), "没有分享入口")
        shot("share-panel")
        share.tap()
        // iOS 26 分享表的活动项是 Cell（attempt2 AX 树：Cell identifier 'actionGroupCell' label 'Save Image'），按设备语言配中英标签。
        guard let save = waitCell(["Save Image", "存储图像"], timeout: 10) else {
            return XCTFail("QR_SHARE_SAVE_CELL_UNAVAILABLE")
        }
        shot("share-sheet")
        save.tap()
        // 全新模拟器第一次保存必弹相册加权限：等面板回归或 Allow/允许，alert 只点一次；弹两个或没有 Allow 都算失败。
        var allowedOnce = false
        var saved = false
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if qrPanelBack(expectedLink) { saved = true; break }
            if app.alerts.firstMatch.exists {
                guard let allow = ["Allow", "允许"].map({ app.alerts.buttons[$0].firstMatch })
                    .first(where: { $0.exists && $0.isHittable }) else { break }
                if allowedOnce { break }
                allow.tap()
                allowedOnce = true
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        XCTAssertTrue(saved, "QR_SHARE_SAVE_RETURN_FAILED")
        shot("after-save")
        share.tap()
        // 先确认分享表真的出现（有 Save Image Cell），再取消；否则可能滑在还没弹出的空屏上。
        guard let sheetItem = waitCell(["Save Image", "存储图像"], timeout: 10) else {
            return XCTFail("QR_SHARE_CANCEL_SHEET_MISSING")
        }
        // iOS 26 分享表 AX 里没有 Cancel/取消 Button（1116 定点实测，10s 轮询无果）：
        // 先按原断言轮询 2s，没有就下滑关闭。
        if let cancel = waitButton(["Cancel", "取消"], timeout: 2), cancel.isEnabled, cancel.isHittable {
            cancel.tap()
        } else {
            sheetItem.swipeDown(velocity: .fast)
        }
        // 取消后分享表要真的关掉：≤5s 内 Save Image Cell 消失，否则算取消不可用。
        XCTAssertTrue(
            app.cells["Save Image"].firstMatch.waitForNonExistence(timeout: 5),
            "QR_SHARE_CANCEL_UNAVAILABLE"
        )
        XCTAssertTrue(waitForQRPanel(expectedLink), "QR_SHARE_CANCEL_RETURN_FAILED")
        shot("after-cancel")
    }

    private func waitForQRPanel(_ expectedLink: String) -> Bool {
        let deadline = Date().addingTimeInterval(10)
        repeat {
            if qrPanelBack(expectedLink) { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        } while Date() < deadline
        return false
    }

    /// 保存/取消后面板要原样回来：链接没变，分享入口仍在、可点。
    private func qrPanelBack(_ expectedLink: String) -> Bool {
        let link = app.staticTexts["friends-my-link"]
        let share = app.buttons["friends-my-share"]
        return app.state == .runningForeground
            && link.exists && link.isHittable && link.label == expectedLink
            && share.exists && share.isEnabled && share.isHittable
    }

    /// 系统分享表的活动项按设备语言可能是英文或中文，逐个标签找（app 内标签仍英文单值）。
    private func waitCell(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for label in labels {
                let cell = app.cells[label].firstMatch
                if cell.exists && cell.isHittable && cell.isEnabled { return cell }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        } while Date() < deadline
        return nil
    }

    func testPastedLinkSendsRequest() throws {
        let peer = try peerId()
        openAddFriend()
        // 二维码里是百分号转义的 matrix.to 用户链接，原样交给核心。
        let encoded = peer.replacingOccurrences(of: "@", with: "%40").replacingOccurrences(of: ":", with: "%3A")
        pasteInScanner("https://matrix.to/#/\(encoded)")
        shot("scan-pasted")
        tapButton("Send Request", timeout: 10)
        XCTAssertTrue(requestArrived(peer), "请求没发出去\n\(app.debugDescription)")
        shot("request-sent")
    }

    /// 对照：同一个人直接在输入框里写用户 ID，和粘贴链接应当得到同样的结果。
    func testTypedIdSendsRequest() throws {
        let peer = try peerId()
        openAddFriend()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "没有输入框")
        field.tap()
        field.typeText(peer)
        tapButton("Send Request", timeout: 10)
        XCTAssertTrue(requestArrived(peer), "请求没发出去\n\(app.debugDescription)")
        shot("typed-request-sent")
    }

    func testNonUserLinkShowsError() throws {
        openAddFriend()
        pasteInScanner("https://matrix.to/#/!room:example.org")
        tapButton("Send Request", timeout: 10)
        XCTAssertTrue(
            app.staticTexts["That is not a Matrix ID"].waitForExistence(timeout: 20),
            "非用户链接没有报错\n\(app.debugDescription)"
        )
        shot("non-user-link")
    }

    /// 请求发出去的两种可见结果：对方是人，好友页出现“等对方同意”；对方是自动接受邀请的 Bot（开发栈里的 wonton-core），
    /// 它立刻加入，聊天列表里多出和它的私聊。
    private func requestArrived(_ peer: String) -> Bool {
        let localpart = String(peer.dropFirst().prefix { $0 != ":" })
        let bot = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", localpart)).firstMatch
        let deadline = Date().addingTimeInterval(60)
        while Date() < deadline {
            if app.staticTexts["Waiting for them to accept"].exists || bot.exists { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return false
    }

    private func peerId() throws -> String {
        let env = ProcessInfo.processInfo.environment
        let peer = env["PEER"] ?? env["TEST_RUNNER_PEER"] ?? ""
        try XCTSkipIf(peer.isEmpty, "没给 TEST_RUNNER_PEER")
        return peer
    }

    /// 打开“扫对方的码”，模拟器不能扫码，把文字粘进粘贴框。
    private func pasteInScanner(_ text: String) {
        tapButton("Scan Their Code", timeout: 10)
        let field = app.textFields["Paste Their Code"].firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "扫码页没有粘贴框")
        field.tap()
        field.typeText(text)
    }

    // MARK: - 走到加好友面板

    private func openAddFriend() {
        guard openSettings() else {
            shot("no-settings")
            return XCTFail("没进主界面\n\(app.debugDescription)")
        }
        // 设置行的读屏标签带着好友数（“Friends, 0”），按开头认；行可能在首屏下面。
        let friends = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Friends")).firstMatch
        let deadline = Date().addingTimeInterval(30)
        while !(friends.exists && friends.isHittable), Date() < deadline {
            if friends.exists { app.swipeUp() } else { _ = friends.waitForExistence(timeout: 2) }
        }
        guard friends.isHittable else { return XCTFail("设置里没有好友入口\n\(app.debugDescription)") }
        friends.tap()
        guard let add = waitButton(["Add Friend", "Add"], timeout: 10) else { return XCTFail("好友页没打开") }
        add.tap()
    }

    /// 配对后走到设置：跳过引导和“在另一台设备继续”，等交接结束，再从侧栏打开设置。最多 90 秒。
    private func openSettings() -> Bool {
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            // ADR-0095：页面入口都在侧栏；引导结束后还可能自动进了一个房间，先退出来。
            let settings = app.buttons["Settings"].firstMatch
            if settings.exists && settings.isHittable {
                settings.tap()
                return true
            }
            let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
            if sidebar.exists && sidebar.isHittable {
                sidebar.tap()
                continue
            }
            let back = app.buttons["Back"].firstMatch
            if back.exists && back.isHittable {
                back.tap()
                continue
            }
            if let skip = ["Later", "Skip", "Done", "Next", "Go to Chat"]
                .map({ app.buttons[$0].firstMatch })
                .first(where: { $0.exists && $0.isHittable && $0.isEnabled }) {
                skip.tap()
            }
            UITestBackend.settleKeyHandoff(app)
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return false
    }

    private func tapButton(_ label: String, timeout: TimeInterval) {
        guard let button = waitButton([label], timeout: timeout) else {
            return XCTFail("没有“\(label)”\n\(app.debugDescription)")
        }
        button.tap()
    }

    private func waitButton(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists && element.isHittable && element.isEnabled { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        } while Date() < deadline
        return nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
