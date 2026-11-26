import XCTest

/// 走查：#661 关于页里的隐私说明和开源许可。
/// 走完引导后从侧边栏进设置 → 关于，截隐私页、许可列表、许可全文；最后用测试钩子再看一次读不到清单的重试态。
final class AboutWalkthroughUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = true
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func button(_ labels: String...) -> XCUIElement? { firstButton(labels) }

    private func button(_ labels: [String]) -> XCUIElement? { firstButton(labels) }

    private func firstButton(_ labels: [String]) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists && element.isHittable { return element }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    private func tap(_ labels: String... , file: StaticString = #filePath, line: UInt = #line) {
        guard let element = button(labels) else {
            XCTFail("找不到 \(labels) \n\(app.debugDescription)", file: file, line: line)
            return
        }
        element.tap()
    }

    private func passOnboarding() {
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            if UITestSidebar.waitForMain(app, timeout: 1) { return }
            if let skip = button("Skip", "跳过"), skip.isEnabled {
                skip.tap()
                continue
            }
            if let done = button("Done", "完成"), done.isEnabled {
                done.tap()
                continue
            }
            if let next = button("Next", "下一步"), next.isEnabled {
                next.tap()
                continue
            }
        }
    }

    private func openAbout() {
        let start = button("Skip", "跳过", "Next", "下一步", "Done", "完成")
            ?? app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(start.waitForExistence(timeout: 60), "没到引导页也没到主界面\n\(app.debugDescription)")
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 240), "没回到主界面")
        UITestSidebar.row(app, labels: ["Settings", "设置"])?.tap()
        XCTAssertNotNil(button("Close"), "设置没打开\n\(app.debugDescription)")
        tap("About", "关于")
        XCTAssertNotNil(button("Privacy", "隐私"), "关于页没打开\n\(app.debugDescription)")
        shot("01-about")
    }

    func testAboutPages() throws {
        continueAfterFailure = false
        openAbout()

        tap("Privacy", "隐私")
        XCTAssertTrue(app.staticTexts["No cloud"].waitForExistence(timeout: 5), "隐私页没打开\n\(app.debugDescription)")
        shot("02-privacy-top")
        for _ in 0..<4 { app.swipeUp() }
        shot("03-privacy-bottom")
        tap("Back", "返回")

        tap("Open source licenses", "开源许可")
        XCTAssertNotNil(button("Full text", "全文"), "许可列表没打开\n\(app.debugDescription)")
        shot("04-licenses")
        tap("Full text", "全文")
        XCTAssertNotNil(button("Back", "返回"))
        shot("05-license-text")
        tap("Back", "返回")
        tap("Back", "返回")
        shot("06-about-again")
    }

    func testLicensesMissing() throws {
        app.terminate()
        app.launchEnvironment["WONTON_LICENSES_MISSING"] = "1"
        UITestBackend.launch(app)
        openAbout()
        tap("Open source licenses", "开源许可")
        XCTAssertNotNil(button("Retry", "重试"), "缺清单时没给重试\n\(app.debugDescription)")
        shot("07-licenses-missing")
    }
}
