import XCTest

/// 还没送达的消息标“已保存，等待送达”（#721 的 iOS 同款）：断网时内容已经存在本机、重启也还在，标的是这件事，
/// 不是“正在上传”。不要后端：`WONTON_UI_FAKE` 的最后一条就是这种待发消息。
/// 浅色、深色各跑一条：App 的外观只按系统走，运行中切设备外观不一定重画，单开一次才拿得准。
final class SendingLabelUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    override func tearDown() {
        XCUIDevice.shared.appearance = .light
    }

    func testQueuedMessageShowsSavedLabelInLight() {
        openQueuedMessage(appearance: .light, shotName: "01-light")
    }

    func testQueuedMessageShowsSavedLabelInDark() {
        openQueuedMessage(appearance: .dark, shotName: "02-dark")
    }

    /// 开 App 进假房间、回到底部，断言最后一条待发消息的时钟和它的说明。
    private func openQueuedMessage(appearance: XCUIDevice.Appearance, shotName: String) {
        XCUIDevice.shared.appearance = appearance
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        UITestBackend.launch(app)

        app.staticTexts["Echo"].tap()
        // 进房停在靠前的消息上，待发的那条在底。正文和发送时间在同一个元素里，按开头找。
        let timeline = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message")).firstMatch
        XCTAssertTrue(timeline.waitForExistence(timeout: 10), "时间线没出来")
        // 落点摆放还在滑时点“向下”会被 BottomPin 当成手指滚动忽略，等它停稳再点；没到底就再点一次。
        Thread.sleep(forTimeInterval: 2)
        let bottom = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Back to bottom")).firstMatch
        XCTAssertTrue(bottom.waitForExistence(timeout: 10), "没有回到底部的按钮")
        bottom.tap()

        // 脚注和时钟可能并进消息的无障碍元素，单独查不到；时钟自己带同一个说明和固定标记。
        let clock = app.descendants(matching: .any)["messageStatusSending"]
        if !clock.waitForExistence(timeout: 8), bottom.exists { bottom.tap() }
        XCTAssertTrue(clock.waitForExistence(timeout: 8), "待发气泡没有时钟\n\(app.debugDescription)")
        XCTAssertEqual(clock.label, "Saved, not delivered yet")
        shot(shotName)
    }

    /// 留在测试进程的临时目录里，由跑测试的脚本从模拟器容器捞出来（测试进程写不了宿主磁盘）。
    private func shot(_ name: String) {
        let png = XCUIScreen.main.screenshot().pngRepresentation
        let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("flow")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? png.write(to: dir.appendingPathComponent("\(name).png"))
    }
}
