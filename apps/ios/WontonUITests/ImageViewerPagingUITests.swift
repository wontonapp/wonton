import Foundation
import XCTest

/// 同一次发送的多张照片在查看器里左右翻：顶栏有页码，滑到头回弹不循环，单张没有页码也不会动；
/// 放大后横拖先滚图片，不翻页。不要后端：用 `WONTON_UI_FAKE` 的假房间（核心的 `ui_fake_room.rs`
/// 第 5–7 条是一组三张、第 3 条是一张单张）。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ImageViewerPagingUITests
final class ImageViewerPagingUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    /// 从第一张翻到最后一张：页码跟着走，到头向右滑回弹，向左滑到第三张停住。
    func testSwipingAndTappingTheSidesTurnsThePage() {
        openPhoto("photo-0.bmp")
        let page = app.staticTexts["viewerPage"]
        XCTAssertTrue(page.waitForExistence(timeout: 10), "一组三张没有页码\n\(app.debugDescription)")
        XCTAssertEqual(page.label, "1 / 3")
        shot("page-1")

        app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.1)
        XCTAssertTrue(wait(5) { page.label == "2 / 3" }, "向左滑没翻到第二张：\(page.label)\n\(app.debugDescription)")
        shot("page-2")

        // 点右侧边也往后翻。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
        XCTAssertTrue(wait(5) { page.label == "3 / 3" }, "点右侧边没翻到第三张：\(page.label)\n\(app.debugDescription)")
        shot("page-3")

        // 最后一张再向左滑，停在原地。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(page.label, "3 / 3", "最后一张向左滑翻出去了")

        // 第一张向右滑，回弹停在第一张。
        tapLeft()
        tapLeft()
        XCTAssertTrue(wait(5) { page.label == "1 / 3" }, "点左侧边没回到第一张：\(page.label)")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(page.label, "1 / 3", "第一张向右滑翻出去了")
        shot("page-1-again")
    }

    /// 双击放大后横拖是滚图片，页码不动；放大时顶栏按钮照常点得到，存一张不动页码。
    func testZoomedDragDoesNotTurnThePageAndSaveKeepsIt() {
        openPhoto("photo-1.bmp")
        let page = app.staticTexts["viewerPage"]
        XCTAssertTrue(page.waitForExistence(timeout: 10), "没有页码")
        XCTAssertEqual(page.label, "2 / 3")

        let center = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))
        center.doubleTap()
        Thread.sleep(forTimeInterval: 1)
        shot("zoomed")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.45))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.45)))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(page.label, "2 / 3", "放大后横拖翻页了")

        // 还原再点保存：保存按钮在顶栏，点它不被当成点边区翻页。
        center.doubleTap()
        Thread.sleep(forTimeInterval: 1)
        let save = app.buttons["Save Photo"].firstMatch
        XCTAssertTrue(save.waitForExistence(timeout: 10), "放大还原后没有保存按钮")
        save.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let alert = springboard.alerts.firstMatch
        if alert.waitForExistence(timeout: 10) {
            let allow = alert.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'Allow'")).firstMatch
            (allow.exists ? allow : alert.buttons.element(boundBy: alert.buttons.count - 1)).tap()
        }
        Thread.sleep(forTimeInterval: 2)
        XCTAssertEqual(page.label, "2 / 3", "保存后页码变了")
        shot("saved-page-2")
    }

    /// 不成组的单张没有页码，左右滑也不动。
    func testSinglePhotoHasNoPageCounter() {
        openPhoto("photo-3.bmp")
        XCTAssertTrue(app.buttons["Save Photo"].firstMatch.waitForExistence(timeout: 15), "单张没打开\n\(app.debugDescription)")
        XCTAssertFalse(app.staticTexts["viewerPage"].exists, "单张也有页码")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertTrue(app.buttons["Save Photo"].firstMatch.exists, "单张左滑后查看器没了")
        shot("single")
    }

    // MARK: 助手

    private func tapLeft() {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5)).tap()
    }

    private func photo(_ name: String) -> XCUIElement {
        app.buttons["messagePhoto.\(name)"]
    }

    /// 进假房间、滚到那张图、点开查看器。
    private func openPhoto(_ name: String) {
        tap(app.buttons[Self.directRoom])
        let bubble = photo(name)
        reveal(bubble)
        bubble.tap()
    }

    /// 整个落在顶栏和输入框之间。
    private func inView(_ element: XCUIElement) -> Bool {
        guard element.exists, element.isHittable else { return false }
        let top = app.staticTexts["barActivity"].frame.maxY
        let bottom = app.textViews["Message"].frame.minY
        return element.frame.minY > top && element.frame.maxY < bottom
    }

    /// 每回进房的位置不定：先一路翻到最新一条，再往回翻着找。撞见就停。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<80 {
            if inView(element) { return }
            dragDown()
        }
        for _ in 0..<80 {
            if inView(element) { return }
            dragUp()
        }
        XCTFail("到底又翻回去 80 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    private func dragUp() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35)))
    }

    private func dragDown() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
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

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
