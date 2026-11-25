import XCTest

/// 你的资料里换、清自己的头像（#662）：系统相册选第一张，上传成功后出现“移除头像”，清掉后消失。
///
/// 模拟器相册要先由外部放一张图（`simctl addmedia`），配对码由 `mini-walkthrough.sh test` 给。
final class OwnerAvatarUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
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

    private func tapWhenHittable(_ element: XCUIElement) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !element.isHittable {
            Thread.sleep(forTimeInterval: 0.5)
        }
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    /// 相册选择器里点第一张图：按坐标点网格左上一格（逐张取元素时快照会翻页，索引查询容易失配）。
    private func tapFirstPhoto() {
        _ = app.staticTexts["Recents"].firstMatch.waitForExistence(timeout: 10)
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.16, dy: 0.62)).tap()
        Thread.sleep(forTimeInterval: 2)
    }

    /// 收掉交接盖层，直到核心明确给出 `settled` 且盖层不再回来；配对没走完时 skip 已经能点，
    /// 所以只认状态，不认按钮出现过。
    private func dismissKeyHandoff() {
        let state = app.descendants(matching: .any)["key-handoff-test-state"].firstMatch
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            if state.exists, state.value as? String == "settled", !app.buttons["key-handoff-skip"].exists {
                break
            }
            let skip = app.buttons["key-handoff-skip"]
            if skip.exists && skip.isHittable { skip.tap() }
            let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
            if back.exists && back.isHittable { back.tap() }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTAssertEqual(state.value as? String, "settled", app.debugDescription)
        XCTAssertFalse(app.buttons["key-handoff-skip"].exists)
    }

    func testPickAndRemoveAvatar() {
        dismissKeyHandoff()
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 240), "没到主界面")
        let pick = app.buttons["Choose Photo"].firstMatch
        var onProfile = false
        for _ in 0..<3 {
            if pick.waitForExistence(timeout: 5) {
                onProfile = true
                break
            }
            // 配对收尾的交接页会在进主界面后才冒出来并挡住点击：关掉，再重开设置。
            _ = UITestBackend.skipKeyHandoff(app)
            UITestSidebar.row(app, labels: ["Settings", "设置", "設定", "設定"])?.tap()
            let profile = app.buttons["Your Profile"].firstMatch
            if profile.waitForExistence(timeout: 10) { profile.tap() }
            if pick.waitForExistence(timeout: 20) {
                onProfile = true
                break
            }
        }
        XCTAssertTrue(onProfile, "资料页没有选图")
        shot("01-profile-before")

        pick.tap()
        tapFirstPhoto()
        let remove = app.buttons["Remove Avatar"].firstMatch
        XCTAssertTrue(remove.waitForExistence(timeout: 90), "头像没设上")
        Thread.sleep(forTimeInterval: 3)
        shot("02-avatar-set")

        remove.tap()
        XCTAssertTrue(remove.waitForNonExistence(timeout: 60), "头像没清掉")
        Thread.sleep(forTimeInterval: 2)
        shot("03-avatar-removed")

        // 留在设上的状态，后面端到端要看另一台设备的聊天头像。
        pick.tap()
        tapFirstPhoto()
        XCTAssertTrue(remove.waitForExistence(timeout: 90), "头像没再设上")
        shot("04-avatar-set-for-e2e")
    }
}
