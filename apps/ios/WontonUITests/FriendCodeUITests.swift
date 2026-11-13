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
        app.launch()
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

    func testPastedLinkSendsRequest() throws {
        let env = ProcessInfo.processInfo.environment
        let peer = env["PEER"] ?? env["TEST_RUNNER_PEER"] ?? ""
        try XCTSkipIf(peer.isEmpty, "没给 TEST_RUNNER_PEER")
        openAddFriend()
        tapButton("Scan Their Code", timeout: 10)

        let field = app.textFields["Paste Their Code"].firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "扫码页没有粘贴框")
        // 二维码里是百分号转义的 matrix.to 用户链接，原样交给核心。
        let encoded = peer.replacingOccurrences(of: "@", with: "%40").replacingOccurrences(of: ":", with: "%3A")
        field.tap()
        field.typeText("https://matrix.to/#/\(encoded)")
        shot("scan-pasted")
        tapButton("Send Request", timeout: 10)

        XCTAssertTrue(
            app.staticTexts["Waiting for them to accept"].waitForExistence(timeout: 40),
            "请求没发出去\n\(app.debugDescription)"
        )
        shot("request-sent")
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

    /// 配对后走到设置：能跳过的引导和“在另一台设备继续”都跳过，设置在侧栏里，侧栏收着就先打开。最多 90 秒。
    private func openSettings() -> Bool {
        let deadline = Date().addingTimeInterval(90)
        var sidebarTapped = Date.distantPast
        while Date() < deadline {
            let settings = app.buttons["Settings"].firstMatch
            if settings.exists && settings.isHittable {
                settings.tap()
                return true
            }
            if let skip = ["Later", "Skip", "Done", "Next"]
                .map({ app.buttons[$0].firstMatch })
                .first(where: { $0.exists && $0.isHittable && $0.isEnabled }) {
                skip.tap()
            } else {
                let sidebar = app.buttons["Sidebar"].firstMatch
                if sidebar.exists && sidebar.isHittable && Date().timeIntervalSince(sidebarTapped) > 6 {
                    sidebar.tap()
                    sidebarTapped = Date()
                }
            }
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
