import XCTest

/// 走查截图：好友码页浅色/深色和取码失败提示。要开发栈（`PAIRING_CODE`），没有就整类跳过。
/// 取码失败靠 `QRImage.make` 的 `WONTON_QR_FAIL` 钩子。
final class FriendCodeShotsUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
    }

    /// 一个配对码只能配一台新设备：第一次启动配对，之后同一个数据目录、去掉码重启。
    /// 外观用 `WONTON_SHOT_SCHEME` 钉住（这台模拟器运行时上系统外观切换不生效）。
    func testMyCodeLightDarkAndFailureHint() throws {
        launch(scheme: "light")
        openMyCode()
        XCTAssertTrue(app.images["My QR Code"].waitForExistence(timeout: 10), "没有二维码")
        let share = app.buttons["friends-my-share"]
        XCTAssertTrue(share.waitForExistence(timeout: 10), "没有分享入口")
        XCTAssertTrue(share.isEnabled && share.isHittable, "分享入口不可用")
        shot("my-code-light")

        app.terminate()
        app.launchEnvironment.removeValue(forKey: "WONTON_DEV_PAIRING_CODE")
        launch(scheme: "dark")
        openMyCode()
        XCTAssertTrue(app.images["My QR Code"].waitForExistence(timeout: 10), "深色下没有二维码")
        let darkShare = app.buttons["friends-my-share"]
        XCTAssertTrue(darkShare.waitForExistence(timeout: 10), "深色下没有分享入口")
        shot("my-code-dark")

        app.terminate()
        launch(scheme: "dark", qrFail: true)
        openMyCode()
        let failure = app.staticTexts["Couldn't create the QR code"]
        XCTAssertTrue(failure.waitForExistence(timeout: 10), "取码失败没有提示\n\(app.debugDescription)")
        XCTAssertFalse(app.images["My QR Code"].exists, "取码失败还画了码")
        XCTAssertTrue(app.staticTexts["friends-my-link"].exists, "失败时链接没了")
        shot("qr-failed-dark")

        app.terminate()
        launch(scheme: "light", qrFail: true)
        openMyCode()
        XCTAssertTrue(failure.waitForExistence(timeout: 10), "浅色下提示不在")
        shot("qr-failed-light")
    }

    private func launch(scheme: String, qrFail: Bool = false) {
        app.launchEnvironment["WONTON_SHOT_SCHEME"] = scheme
        if qrFail {
            app.launchEnvironment["WONTON_QR_FAIL"] = "1"
        } else {
            app.launchEnvironment.removeValue(forKey: "WONTON_QR_FAIL")
        }
        UITestBackend.launch(app)
        UITestBackend.settleKeyHandoff(app)
    }

    private func openMyCode() {
        guard openSettings() else {
            shot("no-settings")
            return XCTFail("没进主界面\n\(app.debugDescription)")
        }
        let friends = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Friends")).firstMatch
        let deadline = Date().addingTimeInterval(30)
        while !(friends.exists && friends.isHittable), Date() < deadline {
            if friends.exists { app.swipeUp() } else { _ = friends.waitForExistence(timeout: 2) }
        }
        guard friends.isHittable else { return XCTFail("设置里没有好友入口\n\(app.debugDescription)") }
        friends.tap()
        guard let add = waitButton(["Add Friend", "Add"], timeout: 10) else { return XCTFail("好友页没打开") }
        add.tap()
        guard let code = waitButton(["My QR Code"], timeout: 40) else { return XCTFail("没有“我的二维码”") }
        code.tap()
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

    private func waitStable(_ element: XCUIElement) -> Bool {
        let deadline = Date().addingTimeInterval(5)
        repeat {
            if element.exists && element.isHittable && element.isEnabled { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        } while Date() < deadline
        return false
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
