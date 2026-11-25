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
        UITestBackend.launch(app)
    }

    /// 附件区里有一张没传上去的图时点发送：核心拒收，原因弹出来，字、回复条和附件原样留着。
    func testRejectedSendKeepsDraftReplyAndAttachment() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
        startReply()
        let field = app.textViews["Message"]
        field.tap()
        field.typeText(Self.draft)
        // 草稿和回复对象先摆好：附件面板选完照片就是发送。
        addFailedAttachment()

        let reason = app.staticTexts["An attachment didn't upload. Retry or remove it first."]
        XCTAssertTrue(reason.waitForExistence(timeout: 5), "被拒了没有说原因\n\(app.debugDescription)")
        screenshot("rejected")
        XCTAssertEqual(field.value as? String, Self.draft, "被拒以后输入框的字没了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复条收了")
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].exists, "被拒以后附件没了")
        XCTAssertFalse(text(Self.draft).exists, "被拒了消息还是发了")

        tap(app.buttons["composerRejectionClose"])
        XCTAssertTrue(reason.waitForNonExistence(timeout: 5), "点掉说明没收起")
        screenshot("kept")
        // 删掉失败的附件再发：这次核心收下，输入框和回复条才清。
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Remove ")).firstMatch)
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].waitForNonExistence(timeout: 5), "删了附件失败标记还在")
        tap(app.buttons["composerSend"])
        XCTAssertTrue(wait(5) { (field.value as? String ?? "").isEmpty }, "收下以后输入框没清空")
        XCTAssertTrue(app.buttons["replyCancel"].waitForNonExistence(timeout: 5), "收下以后回复条没收起")
        XCTAssertTrue(text(Self.draft).waitForExistence(timeout: 5), "收下以后时间线上没有这条")
    }

    /// 附件区里有传不上去的图时点一条命令：命令走发送键同一个口（附件区非空就是 `sendComposer`），
    /// 被核心拒掉、弹出原因；点掉以后附件区还在。
    func testCommandWithFailedAttachmentIsRejected() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
        addFailedAttachment()
        dismissRejection()
        tap(app.buttons["composerCommands"])
        tap(app.buttons["command.help"])

        XCTAssertTrue(
            text("An attachment didn't upload").waitForExistence(timeout: 5),
            "被拒没说原因\n\(app.debugDescription)"
        )
        screenshot("commandRejected")
        dismissRejection()
        tap(app.buttons["composerCommands"])
        XCTAssertFalse(text("/help").exists, "命令没被拦下，还是发出去了")
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].exists, "命令被拒附件没了")
    }

    /// 附件传不上去时点命令：命令没发出去，草稿、回复条和命令列表都留着，处理完附件再点。
    func testRejectedCommandKeepsDraftReplyAndMenu() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
        startReply()
        let field = app.textViews["Message"]
        tap(field)
        field.typeText(Self.draft)
        addFailedAttachment()
        dismissRejection()
        tap(app.buttons["composerCommands"])
        tap(app.buttons["command.model"])
        XCTAssertTrue(
            text("An attachment didn't upload").waitForExistence(timeout: 5),
            "被拒没说原因\n\(app.debugDescription)"
        )
        XCTAssertEqual(field.value as? String, Self.draft, "被拒把草稿清掉了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒把回复条撤了")
        XCTAssertTrue(app.buttons["command.model"].exists, "被拒把命令列表收起来了")
        screenshot("commandRejected")
    }

    /// 附件区里有传不上去的图、敲“/”从列表里点一条：和点“菜单”那条一样，草稿、回复对象和列表都留着；
    /// 处理完附件再点这一条才出去，时间线上只留一条。
    func testRejectedSlashCommandKeepsDraftReplyAndMenu() {
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(message(3).waitForExistence(timeout: 10), "时间线没出来\n\(app.debugDescription)")
        startReply()
        let field = app.textViews["Message"]
        tap(field)
        field.typeText("/h")
        // 放进一张传不上去的图：面板连着草稿发一次被拒，点掉原因，列表照旧。
        addFailedAttachment()
        dismissRejection()
        tap(app.buttons["command.help"])

        XCTAssertTrue(
            text("An attachment didn't upload").waitForExistence(timeout: 5),
            "被拒没说原因\n\(app.debugDescription)"
        )
        XCTAssertEqual(field.value as? String, "/h", "被拒把草稿清掉了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒把回复条撤了")
        XCTAssertTrue(app.buttons["command.help"].exists, "被拒把命令列表收起来了")
        XCTAssertEqual(helpTexts.count, 1, "被拒了命令还是发出去了（列表里那一条之外还有）")
        screenshot("slashCommandRejected")

        dismissRejection()
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Remove ")).firstMatch)
        XCTAssertTrue(app.buttons["composerAttachmentFailed"].waitForNonExistence(timeout: 5), "删了附件失败标记还在")
        tap(app.buttons["command.help"])
        XCTAssertTrue(app.otherElements["commandMenu"].waitForNonExistence(timeout: 5), "收下以后列表没收起")
        XCTAssertEqual(helpTexts.count, 1, "收下以后时间线上没有这条，或者不止一条")
        XCTAssertEqual(field.value as? String, "/h", "点命令把草稿动了")
    }

    // MARK: 助手

    /// 被拒的说明挂在输入栏上方（房间里没有窗口级弹窗）：有就点掉；窗口级弹窗那条路也照样认。
    private func dismissRejection() {
        let close = app.buttons["composerRejectionClose"]
        if close.waitForExistence(timeout: 2) {
            close.tap()
            return
        }
        let ok = app.buttons["dialogOk"]
        if ok.waitForExistence(timeout: 2) { ok.tap() }
    }

    /// 面板里选第一张相册照片，让它被假数据标成上传失败，再照面板的两种收尾发一次：
    /// 新面板底下是发送键，选完点它就连着草稿一起发；只有“完成”的旧面板才点“完成”再走发送键。
    /// 两种都会撞上附件区的失败，调用方紧接着看拒绝就行。
    private func addFailedAttachment() {
        tap(app.buttons["composerAttach"])
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        tap(app.buttons["attachPhoto.0"])
        let sendPicked = app.buttons["attachSend"]
        if sendPicked.waitForExistence(timeout: 2) {
            tap(sendPicked)
        } else {
            tap(app.buttons["attachDone"])
            tap(app.buttons["composerSend"])
        }
        XCTAssertTrue(
            app.buttons["composerAttachmentFailed"].waitForExistence(timeout: 15),
            "附件没被标成上传失败\n\(app.debugDescription)"
        )
    }

    /// 左滑一条消息进入回复。
    private func startReply() {
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

    /// 时间线的消息和菜单行都写“/help”：列表还开着时只能有菜单那一条。
    private var helpTexts: XCUIElementQuery {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "/help"))
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
