import XCTest

/// 配对页能扛住最大系统字号：说明不被顶栏压住、不出屏，码框和提交都还够得到；打的设备名退出 App 后还在。
@MainActor
final class PairingSizeUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 600
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    /// 说明文本：改后带 identifier；改前那棵树没有，按两种句子的开头找。
    private func instructions() -> XCUIElement {
        let byId = element("pairing-instructions")
        if byId.exists { return byId }
        return app.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH %@ OR label BEGINSWITH %@", "Ask whoever", "On your home network")
        ).firstMatch
    }

    private func waitForPairingForm() {
        XCTAssertTrue(element("pairing-device-name").waitForExistence(timeout: 30), app.debugDescription)
    }

    /// 打了一半的设备名退出、重开还在：配对页按返回/被系统收掉都不该丢输入。
    func testDeviceNameSurvivesRelaunch() {
        UITestBackend.launch(app)
        waitForPairingForm()
        let field = element("pairing-device-name")
        field.tap()
        field.typeText("zz-ipad")
        XCTAssertTrue((field.value as? String ?? "").contains("zz-ipad"), "打进去的字没进设备名：\(field.value ?? "")")

        app.terminate()
        UITestBackend.launch(app)
        waitForPairingForm()
        let restored = element("pairing-device-name")
        XCTAssertTrue(
            (restored.value as? String ?? "").contains("zz-ipad"),
            "重开后设备名没了：\(restored.value ?? "")"
        )
    }

    /// 系统字号调到辅助功能最大档：说明整段在屏幕里、不压到顶栏后面，码框、提交、设备名都点得到。
    func testLargestAccessibilitySizeKeepsFormClearOfTopBar() throws {
        app.launchArguments += [
            "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"
        ]
        UITestBackend.launch(app)
        waitForPairingForm()

        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "pairing-largest-text"
        shot.lifetime = .keepAlways
        add(shot)

        // 配对页的三个入口都该是自绘栏（`root-bar`），说明要整段落在栏以下。
        // 改前的树没有这个标识、走的是系统栏：没有标识时退回系统栏底部，量得到哪根栏就用哪根，
        // 改前那条路也能把这组位置断言跑完，只在最后一条“没有自绘栏”上红。
        let window = app.windows.firstMatch.frame
        let instructions = instructions()
        XCTAssertTrue(instructions.exists, app.debugDescription)
        let bar = element("root-bar")
        let navBar = app.navigationBars.firstMatch
        let below = bar.exists
            ? bar.frame.maxY
            : (navBar.exists ? navBar.frame.maxY : window.minY)
        XCTAssertGreaterThanOrEqual(
            instructions.frame.minY, below - 1,
            "说明压到顶栏后面：说明 \(instructions.frame)，顶栏到 \(below)"
        )
        XCTAssertGreaterThan(instructions.frame.height, 20, "说明被压成了一条窄缝")
        XCTAssertGreaterThanOrEqual(instructions.frame.minX, window.minX - 1, "说明出了屏幕左边")
        XCTAssertLessThanOrEqual(instructions.frame.maxX, window.maxX + 1, "说明出了屏幕右边")
        XCTAssertTrue(bar.exists, "配对页没有自绘栏：\(app.debugDescription)")

        // 卡片下半截也不能被截掉：码框和提交滚得到、点得到。
        for id in ["pairing-code", "pairing-submit"] {
            let control = element(id)
            var swipes = 0
            while !control.isHittable && swipes < 8 {
                app.swipeUp()
                swipes += 1
            }
            XCTAssertTrue(control.isHittable, "\(id) 滚不到：\(app.debugDescription)")
            XCTAssertLessThanOrEqual(control.frame.maxY, window.maxY + 1, "\(id) 露到屏幕外")
        }
    }
}
