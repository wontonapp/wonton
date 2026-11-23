import XCTest

/// 核心拒收一次发送时（附件区里有没传上去的附件），字、回复对象和附件都留着，原因如实说出来；
/// 核心收下了才清输入框和回复条。
/// 不要后端：`WONTON_UI_FAKE` 的假房间；`WONTON_UI_TEST_FAIL_ATTACHMENT` 让假数据把放进来的附件标成失败。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ComposerSendRejectUITests
final class ComposerSendRejectUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"
    private static let draft = "send the log first"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_TEST_FAIL_ATTACHMENT"] = "1"
        app.launch()
    }

    /// 附件区里有一张没传上去的图时点发送：核心拒收，原因弹出来，字、回复条和附件原样留着。
    func testRejectedSendKeepsDraftReplyAndAttachment() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
        startReply()
        addFailedAttachment()
        let field = app.textViews["Message"]
        field.tap()
        field.typeText(Self.draft)
        tap(app.buttons["composerSend"])

        let reason = app.staticTexts["An attachment didn't upload. Retry or remove it first."]
        XCTAssertTrue(reason.waitForExistence(timeout: 5), "被拒了没有说原因\n\(app.debugDescription)")
        screenshot("rejected")
        XCTAssertEqual(field.value as? String, Self.draft, "被拒以后输入框的字没了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复条收了")
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].exists, "被拒以后附件没了")
        XCTAssertFalse(text(Self.draft).exists, "被拒了消息还是发了")

        tap(app.buttons["composerRejectionClose"])
        XCTAssertTrue(reason.waitForNonExistence(timeout: 5), "点了关闭，说明没收起")
        screenshot("kept")
        // 删掉失败的附件再发：这次核心收下，输入框和回复条才清。
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Remove ")).firstMatch)
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].waitForNonExistence(timeout: 5), "删了附件失败标记还在")
        tap(app.buttons["composerSend"])
        XCTAssertTrue(wait(5) { (field.value as? String ?? "").isEmpty }, "收下以后输入框没清空")
        XCTAssertTrue(app.buttons["replyCancel"].waitForNonExistence(timeout: 5), "收下以后回复条没收起")
        XCTAssertTrue(text(Self.draft).waitForExistence(timeout: 5), "收下以后时间线上没有这条")
    }

    // MARK: 助手

    /// 回形针里选第一张相册照片，等它被假数据标成上传失败。
    private func addFailedAttachment() {
        tap(app.buttons["composerAttach"])
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        let first = app.buttons["attachPhoto.0"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        tap(first)
        tap(app.buttons["attachSend"])
        XCTAssertTrue(app.buttons["attachSend"].waitForNonExistence(timeout: 5), "选完照片面板没收起")
        XCTAssertTrue(
            app.buttons["composerAttachmentFailed"].waitForExistence(timeout: 15),
            "附件没被标成上传失败\n\(app.debugDescription)"
        )
    }

    /// 左滑一条消息进入回复。
    private func startReply() {
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线里的消息不见了")
        let start = message(3).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(
            forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
            withVelocity: .default, thenHoldForDuration: 0.1
        )
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
    }

    private func message(_ index: Int) -> XCUIElement {
        text("Fixture message \(index)")
    }

    private func text(_ prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    private func tap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        guard wait(20, until: { element.exists && element.isHittable }) else {
            return XCTFail("找不到或点不到 \(element)\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    private func screenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
