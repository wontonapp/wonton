import XCTest

/// 整页的网页让开状态栏和出口胶囊那一条（ADR-0095“铺满”），不要后端：假数据里的 Corner 应用整页打开一张本机网页，
/// 顶上、底下各一条固定的栏，右上角一个固定的按钮（`ui_fake_room.rs` 的 `CORNER`，照 Immich、Actual 的样子摆）。
/// 按钮不在胶囊下面、点得到；滚下去、切后台回来、重新加载以后一样；底下那条栏没被挤出屏幕；
/// 自己写了 `viewport-fit=cover` 的那一份不会让两次。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/PageInsetsUITests
///
/// `obscuredContentInsets` 是 iOS 26 的接口；更早的系统上这一类跳过。
final class PageInsetsUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    override func setUpWithError() throws {
        continueAfterFailure = false
        guard #available(iOS 26, *) else {
            throw XCTSkip("iOS 26 之前没有 obscuredContentInsets，整页不替没适配的网页让位")
        }
    }

    override func tearDown() {
        XCUIDevice.shared.orientation = .portrait
        XCUIDevice.shared.appearance = .light
    }

    func testFixedCornerButtonClearsTheCapsule() {
        launch()
        openCorner()
        assertCornerClear("刚打开")
        shot("corner")

        app.webViews.firstMatch.swipeUp()
        assertCornerClear("滚下去以后")

        backgroundAndReturn()
        assertCornerClear("切后台回来")

        tap(app.buttons.matching(identifier: "pageMore"), "胶囊的菜单")
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "Reload")), "重新加载")
        assertCornerClear("重新加载以后")

        tap(corner, "右上角的固定按钮")
        XCTAssertTrue(
            app.webViews.buttons["Fixture corner tapped"].waitForExistence(timeout: 5),
            "点右上角的固定按钮，网页没收到\n\(app.debugDescription)"
        )
    }

    func testPageThatHandlesSafeAreaItselfDoesNotGiveWayTwice() {
        launch()
        openCorner()
        assertCornerClear("没写 viewport-fit=cover 的页面")
        let plain = cornerTop
        let link = app.webViews.links.matching(NSPredicate(format: "label == %@", "Fixture cover"))
        // 网页刚排好时合成的点按偶尔落空（iOS 26.5 模拟器：页面停在原来那一份，没有发起加载）：没换页就再点一次。
        var loaded = false
        for _ in 0..<3 where !loaded {
            tap(link, "去 cover 那一份的链接")
            loaded = waitProbe(containing: "cover", timeout: 5)
        }
        XCTAssertTrue(loaded, "cover 那一份没加载出来。\(probe)")
        assertCornerClear("写了 viewport-fit=cover 的页面")
        XCTAssertEqual(
            cornerTop, plain, accuracy: 1,
            "写了 viewport-fit=cover 的页面又按安全区让了一次。\(probe)"
        )
        shot("corner-cover")
    }

    func testLargestTextDarkAndLandscape() {
        XCUIDevice.shared.appearance = .dark
        launch(arguments: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        openCorner()
        assertCornerClear("最大字号加深色")
        shot("corner-largest-dark")

        XCUIDevice.shared.orientation = .landscapeLeft
        Thread.sleep(forTimeInterval: 2)
        assertCornerClear("横屏")
        shot("corner-landscape")
        XCUIDevice.shared.orientation = .portrait
        Thread.sleep(forTimeInterval: 2)
        assertCornerClear("转回竖屏")
    }

    // MARK: 助手

    /// 固定在右上角的按钮整个在胶囊下面、点得到；底下那条固定的栏还在屏幕里（视口没被多算一截）。
    private func assertCornerClear(_ when: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(corner), let button = hittable(corner) else {
            return XCTFail("\(when)：右上角的固定按钮不在或点不到。\(probe)\n\(app.debugDescription)", file: file, line: line)
        }
        guard let close = hittable(capsule) else {
            return XCTFail("\(when)：没有点得到的胶囊\n\(app.debugDescription)", file: file, line: line)
        }
        XCTAssertGreaterThanOrEqual(
            button.frame.minY, close.frame.maxY - 1,
            "\(when)：固定按钮 \(button.frame) 还在胶囊 \(close.frame) 那一条里。\(probe)", file: file, line: line
        )
        // 让多了也不对：按钮紧接在胶囊那一行下面。
        XCTAssertLessThanOrEqual(
            button.frame.minY, close.frame.maxY + 16,
            "\(when)：固定按钮 \(button.frame) 离胶囊 \(close.frame) 太远，多让了一截。\(probe)", file: file, line: line
        )
        let foot = app.webViews.staticTexts["Fixture foot"]
        XCTAssertTrue(foot.exists, "\(when)：底下固定的那条栏不在", file: file, line: line)
        XCTAssertLessThanOrEqual(
            foot.frame.maxY, app.frame.maxY + 1,
            "\(when)：底下固定的那条栏 \(foot.frame) 被挤出屏幕 \(app.frame)。\(probe)", file: file, line: line
        )
    }

    private var cornerTop: CGFloat { hittable(corner)?.frame.minY ?? -1 }

    /// 页面自己报的：是哪一份（`plain`、`cover`）、量到的安全区和视口高度，如
    /// `Fixture probe plain safe 0px 0px inner 764 client 764 menu 0`。数值只放进报错里。
    private var probe: String {
        let text = app.webViews.staticTexts
            .matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture probe")).firstMatch
        return text.exists ? text.label : "页面没报"
    }

    private func waitProbe(containing part: String, timeout: TimeInterval = 10) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if probe.contains(part) { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return false
    }

    private func launch(arguments: [String] = []) {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + arguments
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
    }

    private func openCorner(file: StaticString = #filePath, line: UInt = #line) {
        tap(sidebarButton, "侧边栏按钮", file: file, line: line)
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "All")), "侧边栏里的全部应用", file: file, line: line)
        tap(
            app.staticTexts.matching(NSPredicate(format: "label == %@", "Corner")), "网格上的 Corner",
            file: file, line: line
        )
        XCTAssertTrue(
            waitProbe(containing: "plain"), "Corner 的网页没加载出来\n\(app.debugDescription)", file: file, line: line
        )
    }

    /// 按 HOME 再点回来；回到前台的头一瞬间界面还在归位（见 `PluginExitUITests`），等停稳再看。
    private func backgroundAndReturn(file: StaticString = #filePath, line: UInt = #line) {
        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来", file: file, line: line)
        Thread.sleep(forTimeInterval: 1.5)
    }

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    private var capsule: XCUIElementQuery { app.buttons.matching(identifier: "pageClose") }
    private var corner: XCUIElementQuery {
        app.webViews.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture corner"))
    }

    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) != nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) != nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
