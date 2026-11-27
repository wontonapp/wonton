import XCTest

/// 冒烟：App 起得来、主入口进得去、没崩。只断言“到了”，不断言位置、文案、颜色和手势细节；
/// 界面长什么样用截图验收（AGENTS.md“测试”）。不要后端：`WONTON_UI_FAKE` 的假数据。
/// 按名字找元素只是为了点到它，英文界面由启动参数钉住。
final class SmokeUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let echoRoom = "chatRoom.!ui-fake:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        // 每次一个空的数据目录：不吃模拟器上上一轮留下的会话。
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        // 启动后自动弹的通知权限框会吃掉第一个点按。
        app.launchEnvironment["WONTON_DEV_SKIP_PUSH_AUTH"] = "1"
    }

    func testFreshInstallLandsOnPairing() {
        app.launch()
        XCTAssertTrue(
            app.descendants(matching: .any)["pairing-code"].firstMatch.waitForExistence(timeout: 30),
            "全新安装没落在配对页\n\(app.debugDescription)"
        )
    }

    func testLaunchReachesChatList() {
        launchFake()
        XCTAssertTrue(waitHittable(app.buttons[Self.echoRoom]), "没进聊天列表\n\(app.debugDescription)")
    }

    func testSidebarOpens() {
        launchFake()
        openSidebar()
    }

    func testChatSendsAMessage() {
        launchFake()
        tap(app.buttons[Self.echoRoom], "聊天列表里的房间")
        let field = app.textViews["Message"]
        tap(field, "输入框")
        field.typeText("smoke")
        tap(app.buttons["composerSend"], "发送")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "smoke")).firstMatch
                .waitForExistence(timeout: 10),
            "发出去的消息没进时间线\n\(app.debugDescription)"
        )
    }

    func testOpensAnApp() {
        launchFake()
        openSidebar()
        tap(button("All"), "侧边栏里的全部应用")
        let notes = app.staticTexts.matching(NSPredicate(format: "label == %@", "Notes"))
        tap(notes, "应用页里的应用")
        // 点开以后应用页的网格不在了（进了详情页）。
        XCTAssertTrue(waitGone(button("Store")), "点应用没打开\n\(app.debugDescription)")
    }

    func testOpensSettings() {
        launchFake()
        openSidebar()
        tap(button("Settings"), "侧边栏里的设置")
        XCTAssertTrue(waitHittable(button("Appearance")), "设置没打开\n\(app.debugDescription)")
    }

    // MARK: 助手

    private func launchFake() {
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    private func openSidebar() {
        let sidebar = app.buttons.matching(NSPredicate(format: "label == %@", "Sidebar"))
        tap(sidebar, "侧边栏按钮")
        XCTAssertTrue(
            waitHittable(app.staticTexts.matching(NSPredicate(format: "label == %@", "My Apps"))),
            "侧边栏没打开\n\(app.debugDescription)"
        )
    }

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }

    /// 同名的元素里点得到的那个：收起的侧边栏、被盖住的页面还在界面树里。
    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 20) -> Bool {
        poll(timeout) { self.hittable(query) != nil }
    }

    private func waitHittable(_ element: XCUIElement, timeout: TimeInterval = 20) -> Bool {
        poll(timeout) { element.exists && element.isHittable }
    }

    private func waitGone(_ query: XCUIElementQuery, timeout: TimeInterval = 10) -> Bool {
        poll(timeout) { self.hittable(query) == nil }
    }

    private func poll(_ timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return condition()
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func tap(_ element: XCUIElement, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(element) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }
}
