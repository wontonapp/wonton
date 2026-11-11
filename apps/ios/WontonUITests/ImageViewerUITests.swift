import XCTest

/// 图片全屏查看器的走屏：点开、双击缩放、放大拖动、下滑关闭、保存（含权限弹窗和拒绝后的提示）、分享。
/// 驱动真实端到端流程，要本机起好的模拟器、跑着的开发核心和 Echo 发来的大图消息；
/// CI 的模拟器构建只编 Wonton scheme，这两个用例要在本地跑。
/// `testSaveDeniedNotice` 跑之前先把 photos-add 权限复位成未决定。
final class ImageViewerUITests: XCTestCase {
    private var app: XCUIApplication!
    /// 存储属性默认值进不了 MainActor,改成计算属性(XCUIApplication 的 init 是 @MainActor)。
    private var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @discardableResult
    private func waitAnyButton(_ labels: [String], timeout: TimeInterval = 10) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 等底部标签栏（三个标签任意一个）。
    private func tabButton(_ labels: [String]) -> XCUIElement? {
        let height = app.windows.firstMatch.frame.height
        for label in labels {
            for element in app.buttons.matching(identifier: label).allElementsBoundByIndex
            where element.frame.minY > height * 0.7 {
                return element
            }
        }
        return nil
    }

    private func waitForTabs(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if tabButton(["Apps", "应用", "Chats", "聊天", "Settings", "设置"]) != nil { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return false
    }

    private func passOnboarding() {
        for _ in 0..<12 {
            if waitForTabs(timeout: 1) { return }
            if let enter = waitAnyButton(["Enter Chat", "进入聊天"], timeout: 2), enter.isEnabled {
                enter.tap()
                continue
            }
            if let skip = waitAnyButton(["Skip", "跳过"], timeout: 2), skip.isEnabled {
                skip.tap()
                continue
            }
            if let done = waitAnyButton(["Done", "完成"], timeout: 2), done.isEnabled {
                done.tap()
                continue
            }
            if let next = waitAnyButton(["Next", "下一步"], timeout: 2), next.isEnabled {
                next.tap()
                continue
            }
        }
    }

    /// 应用页上 Echo 的卡片。
    private func echoTile() -> XCUIElement? {
        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            for name in ["回声", "Echo"] {
                let byLabel = app.buttons.containing(.staticText, identifier: name).firstMatch
                if byLabel.exists { return byLabel }
                let text = app.staticTexts[name].firstMatch
                if text.exists { return text }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 聊天里的大图气泡：气泡本身是按钮，宽度超过一屏的一半。
    private func imageBubble() -> XCUIElement? {
        let window = app.windows.firstMatch.frame
        let bubble = app.buttons.allElementsBoundByIndex.first {
            $0.frame.width > window.width * 0.5 && $0.frame.height > window.width * 0.5 && $0.frame.minY > 80
        }
        return bubble
    }

    func testImageViewerFlow() throws {
        // 冷启动可能是三种界面：主界面、引导页、或上次运行留下的聊天。
        if app.buttons["返回"].waitForExistence(timeout: 3) {
            app.buttons["返回"].firstMatch.tap()
        }
        let start = waitAnyButton(["Apps", "应用"], timeout: 25)
            ?? waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成", "Enter Chat", "进入聊天"], timeout: 90)
        XCTAssertNotNil(start, "既没到主界面也没到引导页")
        passOnboarding()
        // 引导最后一步"进入聊天"直达聊天页，退回主界面。
        if !waitForTabs(timeout: 3), app.buttons["返回"].firstMatch.exists {
            app.buttons["返回"].firstMatch.tap()
        }
        XCTAssertTrue(waitForTabs(timeout: 240), "没回到主界面")

        // 去应用页，点开和 Echo 的私聊。
        tabButton(["Apps", "应用"])?.tap()
        let tile = echoTile()
        XCTAssertNotNil(tile, "应用页没有 Echo 卡片")
        tile?.tap()

        // 等机器人发来的大图出现（原图 3000×4000，气泡过半屏宽）。
        let deadline = Date().addingTimeInterval(60)
        var bubble: XCUIElement?
        while Date() < deadline, bubble == nil {
            bubble = imageBubble()
            if bubble == nil { _ = app.wait(for: .runningForeground, timeout: 2) }
        }
        guard let bubble else {
            shot("fail-no-image")
            XCTFail("聊天里没有大图消息")
            return
        }
        shot("01-room")

        // 点开全屏：等原图下载完（保存按钮出现）。
        bubble.tap()
        let save = waitAnyButton(["Save Photo", "存储图像"], timeout: 60)
        XCTAssertNotNil(save, "查看器没打开或原图没下载完")
        shot("02-viewer-loaded")

        // 查看器没有 ScrollView，手势直接落在全屏坐标上。
        let center = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))

        // 双击放大、再双击还原。
        center.doubleTap()
        sleep(1)
        shot("03-zoomed-in")
        center.doubleTap()
        sleep(1)

        // 双击放大后拖动。
        center.doubleTap()
        sleep(1)
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
        from.press(forDuration: 0.05, thenDragTo: to)
        sleep(1)
        shot("04-panned")
        center.doubleTap()
        sleep(1)

        // 下滑关闭：从上部往下滑到底。
        let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        let bottom = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
        top.press(forDuration: 0.05, thenDragTo: bottom)
        let closed = app.buttons["附件"].waitForExistence(timeout: 8)
        XCTAssertTrue(closed, "下滑后没有回到聊天")
        shot("05-closed-by-swipe")

        // 再打开，保存进相册：第一次弹系统权限，允许后存入。
        bubble.tap()
        XCTAssertNotNil(waitAnyButton(["Save Photo", "存储图像"], timeout: 60), "第二次打开没看到原图")
        waitAnyButton(["Save Photo", "存储图像"])?.tap()
        let allow = springboard.buttons["Allow"].waitForExistence(timeout: 10)
            ? springboard.buttons["Allow"]
            : springboard.buttons["允许"]
        if allow.exists {
            shot("06-photo-permission")
            allow.tap()
            sleep(2)
            shot("07-saved")
        } else {
            // 没弹（权限已在系统设置里给过）：存完应有对勾。
            sleep(2)
            shot("07-saved-no-prompt")
        }

        // 分享面板。
        waitAnyButton(["Share", "分享"])?.tap()
        sleep(2)
        shot("08-share-sheet")
        app.swipeDown()
        sleep(1)
    }

    /// 没权限时的如实提示：拒掉 photos-add 再点保存，看提示和“打开设置”。
    func testSaveDeniedNotice() throws {
        // 冷启动可能是三种界面：主界面、引导页、或上次运行留下的聊天。
        if app.buttons["返回"].waitForExistence(timeout: 3) {
            app.buttons["返回"].firstMatch.tap()
        }
        // 冷启动直接到主界面（已配对、已过引导）。
        let start = waitAnyButton(["Apps", "应用"], timeout: 25)
            ?? waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成", "Enter Chat", "进入聊天"], timeout: 90)
        XCTAssertNotNil(start)
        passOnboarding()
        XCTAssertTrue(waitForTabs(timeout: 240), "没回到主界面")
        tabButton(["Apps", "应用"])?.tap()
        echoTile()?.tap()

        let deadline = Date().addingTimeInterval(60)
        var bubble: XCUIElement?
        while Date() < deadline, bubble == nil {
            bubble = imageBubble()
            if bubble == nil { _ = app.wait(for: .runningForeground, timeout: 2) }
        }
        guard let bubble else {
            shot("fail-no-image-denied")
            XCTFail("聊天里没有大图消息")
            return
        }
        bubble.tap()
        let save = waitAnyButton(["Save Photo", "存储图像"], timeout: 60)
        XCTAssertNotNil(save)
        shot("09a-viewer-before-save")
        save?.tap()
        sleep(2)
        shot("09b-after-save-tap")
        // 系统若再弹权限，选拒绝。
        let dontAllow = springboard.buttons["Don't Allow"].waitForExistence(timeout: 8)
            ? springboard.buttons["Don't Allow"]
            : springboard.buttons["不允许"]
        shot("09c-prompt-state")
        dontAllow.tap()
        sleep(2)
        shot("09d-after-deny")
        let notice = waitAnyButton(["Open Settings", "打开设置"], timeout: 25)
        shot("09e-notice-state")
        XCTAssertNotNil(notice, "被拒后没有出现“打开设置”的提示")
        shot("09-denied-notice")
    }
}
