import XCTest
import UIKit

@MainActor
final class ContentWindowWalkthroughTests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 300
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
        try UITestBackend.passPairingCode(to: app)
        addUIInterruptionMonitor(withDescription: "Local network") { alert in
            for label in ["允许", "Allow", "好", "OK"] {
                if alert.buttons[label].exists { alert.buttons[label].tap(); return true }
            }
            return false
        }
    }

    func testContentWindowFromConversationMenu() {
        app.launchEnvironment.removeValue(forKey: "WONTON_DEV_PAIRING_CODE")
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        XCUIDevice.shared.orientation = .landscapeLeft
        app.launch()
        XCTAssertTrue(waitForMain(), app.debugDescription)
        let room = firstRoom()
        XCTAssertTrue(room.waitForExistence(timeout: 10), app.debugDescription)
        room.press(forDuration: 1)
        let open = button(["在新窗口打开", "Open in New Window"])
        XCTAssertTrue(open.waitForExistence(timeout: 10), app.debugDescription)
        open.tap()
        XCTAssertTrue(app.textViews.firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertFalse(button(["返回", "Back"]).isHittable, app.debugDescription)
        shot("content-room-scene-surface")
    }

    func testRoomWindowSharesDraftAndRestoresTarget() throws {
        try UITestBackend.require()
        XCTAssertEqual(UIDevice.current.userInterfaceIdiom, .pad)
        XCUIDevice.shared.orientation = .landscapeLeft
        app.launch()
        XCTAssertTrue(waitForMain(), app.debugDescription)
        let room = firstRoom()
        XCTAssertTrue(room.waitForExistence(timeout: 90), app.debugDescription)
        room.tap()
        let input = app.textViews.firstMatch
        XCTAssertTrue(input.waitForExistence(timeout: 10), app.debugDescription)
        input.tap()
        input.typeText("shared scene draft")
        app.swipeDown()
        firstRoom().press(forDuration: 1)
        let open = button(["在新窗口打开", "Open in New Window"])
        XCTAssertTrue(open.waitForExistence(timeout: 10), app.debugDescription)
        open.tap()
        XCTAssertTrue(waitUntil {
            self.app.textViews.allElementsBoundByIndex.filter {
                ($0.value as? String) == "shared scene draft"
            }.count >= 2
        }, app.debugDescription)
        shot("content-room-two-windows")
        let inputs = app.textViews.allElementsBoundByIndex.filter { ($0.value as? String) == "shared scene draft" }
        let other = try XCTUnwrap(inputs.last)
        other.tap()
        other.typeText(" updated")
        XCTAssertTrue(waitUntil {
            self.app.textViews.allElementsBoundByIndex.filter {
                ($0.value as? String) == "shared scene draft updated"
            }.count >= 2
        }, app.debugDescription)
        shot("content-room-shared-draft")
        app.launchEnvironment.removeValue(forKey: "WONTON_DEV_PAIRING_CODE")
        app.terminate()
        app.activate()
        XCTAssertTrue(app.textViews.firstMatch.waitForExistence(timeout: 90), app.debugDescription)
        XCTAssertTrue(app.textViews.allElementsBoundByIndex.contains {
            ($0.value as? String) == "shared scene draft updated"
        }, app.debugDescription)
        shot("content-room-restored")
    }

    func testPluginContentCloseLeavesMainPageOpen() throws {
        try UITestBackend.require()
        XCUIDevice.shared.orientation = .landscapeLeft
        app.launch()
        XCTAssertTrue(waitForMain(), app.debugDescription)
        button(["我的应用", "My Apps"]).tap()
        var page = button(["回显", "Who am I"])
        if !page.exists {
            button(["全部", "All", "全部应用", "All Apps"]).tap()
            page = button(["回显", "Who am I"])
        }
        XCTAssertTrue(page.waitForExistence(timeout: 10), app.debugDescription)
        page.tap()
        XCTAssertTrue(app.webViews.firstMatch.waitForExistence(timeout: 90), app.debugDescription)
        button(["更多", "More"]).tap()
        button(["在新窗口打开", "Open in New Window"]).tap()
        XCTAssertTrue(waitUntil { self.app.webViews.count >= 2 }, app.debugDescription)
        shot("content-plugin-two-windows")
        let closeButtons = app.buttons.allElementsBoundByIndex.filter {
            ["关闭", "Close"].contains($0.label) && $0.isHittable
        }
        try XCTUnwrap(closeButtons.last).tap()
        XCTAssertTrue(app.staticTexts["页面已关闭"].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.webViews.firstMatch.exists, app.debugDescription)
        shot("content-plugin-local-close")
    }

    private func firstRoom() -> XCUIElement {
        app.buttons.allElementsBoundByIndex.first {
            ($0.label.contains("回声") || $0.label.contains("Echo")) && $0.isHittable
        } ?? app.staticTexts["回声"].firstMatch
    }

    private func button(_ labels: [String]) -> XCUIElement {
        for label in labels {
            let candidate = app.buttons[label].firstMatch
            if candidate.exists && candidate.isHittable { return candidate }
        }
        return app.buttons[labels[0]].firstMatch
    }

    private func waitForMain() -> Bool {
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            if UITestBackend.skipKeyHandoff(app) { continue }
            if button(["我的应用", "My Apps"]).exists { return true }
            let next = button(["跳过", "Skip", "进入聊天", "Go to Chat", "完成", "Done", "下一步", "Next"])
            if next.exists && next.isEnabled && next.isHittable { next.tap() }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return false
    }

    private func waitUntil(_ predicate: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if predicate() { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return predicate()
    }

    private func shot(_ name: String) {
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let tree = XCTAttachment(string: app.debugDescription)
        tree.name = "\(name)-accessibility"
        tree.lifetime = .keepAlways
        add(tree)
    }
}
