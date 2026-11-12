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
        UITestBackend.passPairingCode(to: app)
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
        let peer = ProcessInfo.processInfo.environment["TEST_RUNNER_PEER"] ?? ""
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
        passOnboarding()
        guard let settings = tabButton(["Settings"]) else {
            shot("no-tab-bar")
            return XCTFail("没进主界面\n\(app.debugDescription)")
        }
        settings.tap()
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

    /// 配对后走完引导：能跳过的跳过。整段最多 60 秒（含配对和首次同步）。
    private func passOnboarding() {
        let deadline = Date().addingTimeInterval(60)
        while Date() < deadline {
            if tabButton(["Chats", "Apps"]) != nil { return }
            for label in ["Skip", "Done", "Next"] {
                let button = app.buttons[label].firstMatch
                if button.exists && button.isHittable && button.isEnabled {
                    button.tap()
                    break
                }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
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

    /// 底栏的按钮：在屏幕下部，和页面里同名的按钮分开。
    private func tabButton(_ labels: [String]) -> XCUIElement? {
        let height = app.windows.firstMatch.frame.height
        for label in labels {
            for element in app.buttons.matching(identifier: label).allElementsBoundByIndex
            where element.frame.minY > height * 0.7 { return element }
        }
        return nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
