import XCTest

/// 开发栈装 whoami 并声明 TCP 映射；PORT_MAPPING_TEST_PORT 给出端口，用真实请求验证监听生命周期。
@MainActor
final class PortMappingUITests: XCTestCase {
    private var app: XCUIApplication!
    private var port: UInt16 = 0

    func testPageAndForegroundControlMapping() async throws {
        continueAfterFailure = false
        executionTimeAllowance = 300
        try UITestBackend.require()
        let configured = ProcessInfo.processInfo.environment["PORT_MAPPING_TEST_PORT"]
        try XCTSkipIf(configured == nil, "Requires the whoami TCP mapping fixture and PORT_MAPPING_TEST_PORT")
        port = try XCTUnwrap(UInt16(configured ?? ""))
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        try UITestBackend.passPairingCode(to: app)
        UITestBackend.launch(app)
        // 先等核心给交接结论（配对没完成时盖层上的 Skip 点了不会落到备份页），再关本机引导。
        dismissKeyHandoff()
        passOnboarding()
        waitForRoot()
        XCTAssertFalse(app.buttons["key-handoff-skip"].exists)
        try await expectMapping(false)

        try openPluginPage()
        try await expectMapping(true)

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(
            app.wait(for: .runningBackground, timeout: 10) || app.state == .runningBackgroundSuspended
        )
        try await expectMapping(false)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10))
        try openPluginPage()
        try await expectMapping(true)

        app.buttons["More"].tap()
        let details = app.buttons["View Details"]
        XCTAssertTrue(details.waitForExistence(timeout: 10), app.debugDescription)
        details.tap()
        let state = app.descendants(matching: .any)["portMappingState"].firstMatch
        XCTAssertTrue(state.waitForExistence(timeout: 90), app.debugDescription)
        for _ in 0..<8 where !state.isHittable { app.swipeUp() }
        XCTAssertTrue(state.isHittable)
        XCTAssertFalse(app.descendants(matching: .any)["portMappingReason"].exists)
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "ios-port-map-active"
        attachment.lifetime = .keepAlways
        add(attachment)

        app.buttons["Back"].tap()
        let close = app.buttons["Close"].firstMatch
        XCTAssertTrue(close.waitForExistence(timeout: 10), app.debugDescription)
        close.tap()
        try await expectMapping(false)
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 10), app.debugDescription)
        XCTAssertFalse(app.webViews.firstMatch.isHittable)
    }

    /// 收掉交接盖层：Skip 落不落到备份页都行，只要求核心最终给出 `settled`、盖层不再回来。
    /// 配对还没走完时 `key-handoff-skip` 就已经可点，点了备份页可能来不及出现，所以不像
    /// `UITestBackend.settleKeyHandoff` 那样硬断言备份页。
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
        let back = app.buttons.matching(NSPredicate(format: "label IN %@", ["Back", "返回"])).firstMatch
        if back.exists && back.isHittable { back.tap() }
        XCTAssertEqual(state.value as? String, "settled", app.debugDescription)
        XCTAssertFalse(app.buttons["key-handoff-skip"].exists)
    }

    /// 走完引导或跳过。全新安装（每次 `up` 都 erase）会先落在引导页，页上没有侧边栏按钮。
    private func passOnboarding() {
        for _ in 0..<12 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            // 引导是独立的 5 步页；走完就落进默认聊天，不是根页面。
            if !app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Step '")).firstMatch.exists { return }
            // 最后一页的「跳过」是禁用的，要看「完成」「进入聊天」。
            if let button = waitAnyButton(["Done", "完成", "Go to Chat", "进入聊天"], timeout: 2), button.isEnabled {
                button.tap()
                continue
            }
            if let button = waitAnyButton(["Next", "下一步"], timeout: 2), button.isEnabled {
                button.tap()
                continue
            }
            if let button = waitAnyButton(["Skip", "跳过"], timeout: 2), button.isEnabled {
                button.tap()
            }
        }
    }

    /// 根页面（底栏没了以后，根页面才有侧边栏按钮）。引导走完会停在默认聊天里，先退出去。
    private func waitForRoot(timeout: TimeInterval = 60) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if app.buttons.matching(UITestSidebar.button).firstMatch.exists { return }
            let back = app.buttons["Back"]
            if back.exists && back.isHittable { back.tap() }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("没回到根页面：\(app.debugDescription)")
    }

    private func waitAnyButton(_ labels: [String], timeout: TimeInterval) -> XCUIElement? {
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

    /// 打开插件页。交接盖层迟到时 `SkipKeyHandoff` 的 `openBackup()` 会把页面栈切到 `ChatBackup`，
    /// 插件页连同映射一起消失；所以每次开好都等一层 `settle`，被顶掉就重开，不拿“当时的栈”当起点。
    private func openPluginPage() throws {
        let url = try XCTUnwrap(URL(string: "wonton://tab/plugin/app.whoami"))
        // 深链在导航没就绪时被静默丢掉（不排队重试），所以每次重开、只等一小段。
        for _ in 0..<6 {
            app.open(url)
            guard app.webViews.firstMatch.waitForExistence(timeout: 20) else { continue }
            UITestBackend.settleKeyHandoff(app, timeout: 30)
            if app.webViews.firstMatch.isHittable { return }
        }
        XCTFail("插件页反复被交接盖层顶掉：\(app.debugDescription)")
    }

    private func expectMapping(_ available: Bool) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 1
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/"))
        let deadline = Date().addingTimeInterval(10)
        var reached = false
        repeat {
            do {
                let (data, response) = try await session.data(from: url)
                reached = (response as? HTTPURLResponse)?.statusCode == 200
                    && (String(bytes: data, encoding: .utf8) ?? "").contains("GET / HTTP/")
            } catch {
                reached = false
            }
            if reached == available { return }
            try await Task.sleep(for: .milliseconds(200))
        } while Date() < deadline
        XCTFail("Expected local mapping availability \(available), got \(reached)")
    }
}
