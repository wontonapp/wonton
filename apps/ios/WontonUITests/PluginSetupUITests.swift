import XCTest

/// 详情页上的首次打开步骤（`x-wonton.setup`）：装在描述下面，不用点开安装才看得到。
/// 官方假核心的布局取证，不需要后端；用 -only-testing:WontonUITests/PluginSetupUITests 跑。
final class PluginSetupUITests: XCTestCase {
    private var app: XCUIApplication!
    private static let heading = "First time opening it"
    private static let step = "Sign in as admin, then change the password."

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    func testSetupShowsUnderTheDescription() {
        tap(app.buttons.matching(UITestSidebar.button), "Sidebar")
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "All")), "All apps")
        tap(app.staticTexts.matching(NSPredicate(format: "label == %@", "Home Hub")), "Home Hub", press: true)
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "Details")), "Details in the Home Hub menu")

        let heading = app.staticTexts[Self.heading]
        let step = app.staticTexts[Self.step]
        XCTAssertTrue(heading.waitForExistence(timeout: 8), app.debugDescription)
        reveal(step)
        XCTAssertLessThan(heading.frame.minY, step.frame.minY, "Setup is not laid out as a section")
        XCTAssertWithin(app.frame, step)

        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "UI-FAKE LAYOUT ONLY-plugin-setup"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func XCTAssertWithin(_ screen: CGRect, _ element: XCUIElement) {
        XCTAssertGreaterThanOrEqual(element.frame.minX, screen.minX)
        XCTAssertLessThanOrEqual(element.frame.maxX, screen.maxX)
    }

    private func reveal(_ element: XCUIElement) {
        for _ in 0..<8 {
            if element.exists && element.isHittable { return }
            app.swipeUp()
        }
        XCTAssertTrue(element.exists && element.isHittable, "Element did not become visible\n\(app.debugDescription)")
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, press: Bool = false) {
        let deadline = Date().addingTimeInterval(8)
        repeat {
            if let element = query.allElementsBoundByIndex.first(where: {
                $0.exists && $0.isHittable && $0.isEnabled
            }) {
                if press { element.press(forDuration: 1) } else { element.tap() }
                return
            }
            Thread.sleep(forTimeInterval: 0.2)
        } while Date() < deadline
        XCTFail("\(name) is not tappable\n\(app.debugDescription)")
    }
}
