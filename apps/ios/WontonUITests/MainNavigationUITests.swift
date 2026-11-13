import XCTest

/// 主入口的导航：从侧边栏进每一页，每页的返回（左边缘右滑、返回键）和顶栏按钮都得管用。
///
/// 装到真机前的必走清单，一条命令跑完（不要后端，用 `WONTON_UI_FAKE` 的假数据）：
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/MainNavigationUITests
///
/// 整页插件页（网页）的出口要连开发栈，在 `PluginPageUITests`；聊天页里面的手势在 `ChatPanelUITests`。
final class MainNavigationUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
    }

    // MARK: 应用页

    func testAppsEdgeSwipeReturnsToChat() {
        openApps()
        shot("apps")
        edgeSwipeBack()
        assertOnChatRoot("应用页左边缘右滑")
    }

    func testAppsBackButtonReturnsToChat() {
        openApps()
        tap(button("Back"), "应用页的返回键")
        assertOnChatRoot("应用页点返回")
    }

    func testAppsSettingsButtonOpensSettings() {
        openApps()
        tap(button("Settings"), "应用页右上角的设置")
        XCTAssertTrue(waitHittable(settingsClose), "应用页点设置没打开设置\n\(app.debugDescription)")
        shot("apps-settings")
        tap(settingsClose, "设置的关闭")
        XCTAssertTrue(waitHittable(appsPage), "关掉设置没回到应用页\n\(app.debugDescription)")
        edgeSwipeBack()
        assertOnChatRoot("关掉设置后应用页左边缘右滑")
    }

    func testAppsStoreEntryAndPluginDetail() {
        openApps()
        tap(button("Store"), "应用页的商店入口")
        XCTAssertTrue(waitHittable(storeTitle), "商店入口没进商店\n\(app.debugDescription)")
        tap(button("Back"), "商店的返回键")
        XCTAssertTrue(waitHittable(appsPage), "商店点返回没回应用页\n\(app.debugDescription)")

        // 假数据里的 Notes 没有网页也没有 Bot：点图标进详情页。详情页上没有齿轮，拿它认回没回应用页。
        let notes = text("Notes")
        let gear = button("Settings")
        tap(notes, "应用图标")
        XCTAssertTrue(waitGone(gear), "点应用图标没进详情页\n\(app.debugDescription)")
        shot("plugin-detail")
        edgeSwipeBack()
        XCTAssertTrue(waitHittable(gear), "详情页左边缘右滑没回应用页\n\(app.debugDescription)")

        tap(notes, "应用图标")
        XCTAssertTrue(waitGone(gear), "第二次点应用图标没进详情页\n\(app.debugDescription)")
        tap(button("Back"), "详情页的返回键")
        XCTAssertTrue(waitHittable(gear), "详情页点返回没回应用页\n\(app.debugDescription)")
    }

    // MARK: 商店

    func testStoreFromSidebar() {
        openFromSidebar("Store", "商店")
        XCTAssertTrue(waitHittable(storeTitle), "侧边栏的商店没进商店\n\(app.debugDescription)")
        shot("store")

        tap(button("Add Source…"), "商店右上角的添加来源")
        let close = button("Close")
        XCTAssertTrue(waitHittable(close), "添加来源没打开\n\(app.debugDescription)")
        tap(close, "添加来源的关闭")
        XCTAssertTrue(waitHittable(storeTitle), "关掉添加来源没回商店\n\(app.debugDescription)")

        // 商店下面是应用页：返回一层到应用页，再一层回聊天。
        edgeSwipeBack()
        XCTAssertTrue(waitHittable(appsPage), "商店左边缘右滑没回应用页\n\(app.debugDescription)")
        edgeSwipeBack()
        assertOnChatRoot("从商店退两层")
    }

    // MARK: 设置

    func testSettingsFromSidebar() {
        openFromSidebar("Settings", "设置")
        XCTAssertTrue(waitHittable(settingsClose), "侧边栏的设置没打开\n\(app.debugDescription)")
        shot("settings")

        let appearance = button("Appearance")
        let subpageBack = button("Back")
        tap(appearance, "设置里的外观")
        XCTAssertTrue(waitHittable(subpageBack), "外观页没打开\n\(app.debugDescription)")
        edgeSwipeBack()
        XCTAssertTrue(waitHittable(settingsClose), "设置的下一层左边缘右滑没返回\n\(app.debugDescription)")

        tap(appearance, "设置里的外观")
        tap(subpageBack, "外观页的返回键")
        XCTAssertTrue(waitHittable(settingsClose), "设置的下一层点返回没返回\n\(app.debugDescription)")

        tap(settingsClose, "设置的关闭")
        assertOnChatRoot("关掉设置")
    }

    // MARK: 聊天

    func testChatRoomBack() {
        let room = text("Echo")
        let back = button("Back")
        tap(room, "聊天列表里的房间")
        XCTAssertTrue(waitHittable(back), "没进聊天页\n\(app.debugDescription)")
        shot("room")
        tap(back, "聊天页的返回键")
        assertOnChatRoot("聊天页点返回")

        tap(room, "聊天列表里的房间")
        XCTAssertTrue(waitHittable(back), "第二次没进聊天页\n\(app.debugDescription)")
        edgeSwipeBack()
        assertOnChatRoot("聊天页左边缘右滑")
    }

    func testNewChatFromSidebarAndChatList() {
        let title = text("Start a Chat")
        openFromSidebar("New Chat", "新聊天")
        XCTAssertTrue(waitHittable(title), "侧边栏的新聊天没打开\n\(app.debugDescription)")
        tap(button("Back"), "新聊天页的返回键")
        assertOnChatRoot("新聊天页点返回")

        tap(button("New Chat"), "聊天列表的新聊天按钮")
        XCTAssertTrue(waitHittable(title), "聊天列表的新聊天按钮没打开\n\(app.debugDescription)")
        edgeSwipeBack()
        assertOnChatRoot("新聊天页左边缘右滑")
    }

    // MARK: 侧边栏

    func testSidebarChatsRowReturnsToChatList() {
        openFromSidebar("Chats", "聊天")
        assertOnChatRoot("侧边栏点聊天")

        // 从应用页回来以后，根页面的左边缘右滑还是开侧边栏。
        openApps()
        edgeSwipeBack()
        assertOnChatRoot("应用页左边缘右滑")
        edgeSwipeBack()
        XCTAssertTrue(waitHittable(text("My Apps")), "回到聊天后左边缘右滑没开侧边栏\n\(app.debugDescription)")
        tap(button("Collapse sidebar"), "侧边栏露出的那一条")
        assertOnChatRoot("收起侧边栏")
    }

    // MARK: 助手

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    /// 应用页到了：网格上假数据里的那个应用看得见。
    private var appsPage: XCUIElementQuery { text("Notes") }
    private var storeTitle: XCUIElementQuery { text("Store") }
    /// 设置首页右上角的关闭：设置是主界面上的整屏盖子。
    private var settingsClose: XCUIElementQuery { button("Close") }

    private func openApps(file: StaticString = #filePath, line: UInt = #line) {
        // “我的应用”标题旁的“全部”一直在；“全部应用”那一行要有没加进“我的应用”的应用才有。
        openFromSidebar("All", "全部应用", file: file, line: line)
        XCTAssertTrue(waitHittable(appsPage), "没进应用页\n\(app.debugDescription)", file: file, line: line)
    }

    /// 先开侧边栏再找：聊天列表上也有同名的按钮（新聊天），只认侧边栏里点得到的那个。
    private func openFromSidebar(
        _ label: String, _ name: String, file: StaticString = #filePath, line: UInt = #line
    ) {
        tap(sidebarButton, "侧边栏按钮", file: file, line: line)
        XCTAssertTrue(
            waitHittable(text("My Apps")), "侧边栏没打开\n\(app.debugDescription)",
            file: file, line: line
        )
        tap(button(label), "侧边栏里的\(name)", file: file, line: line)
    }

    /// 回到了聊天根页面：侧边栏按钮看得见、点得到（盖着别的层时它点不到或不在）。
    private func assertOnChatRoot(_ after: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(
            waitHittable(sidebarButton), "\(after)之后没回到聊天\n\(app.debugDescription)", file: file, line: line
        )
    }

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }

    private func text(_ label: String) -> XCUIElementQuery {
        app.staticTexts.matching(NSPredicate(format: "label == %@", label))
    }

    /// 同名的元素里点得到的那个：收起的侧边栏、被盖住的页面还在界面树里，按名字取第一个会取到它们。
    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    /// 和真机上一样的返回手势：从屏幕左边缘往右拖。上一个转场（推入、返回、盖子收起）还在走时系统不认返回手势，
    /// 所以先等画面停下来。辅助功能给的位置是终值，看不出转场走没走完，只能看屏幕。
    private func edgeSwipeBack(file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(waitTopBarStill(), "顶栏一直在动，转场没走完\n\(app.debugDescription)", file: file, line: line)
        // 拖到头停一下再松手：松手时速度是零，返回成不成只看拖过的距离（过半就成）。不停的话要看松手那一刻
        // 算出来的速度，模拟器一卡，拖到一半的转场会弹回去（录屏里看得到）。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 连着两次截屏，顶栏那一条一模一样才算停稳。只比顶栏：状态栏的时钟、页面里的转圈会自己变。
    private func waitTopBarStill(timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        var last: Data?
        while Date() < deadline {
            let bar = topBarPixels()
            if let bar, bar == last { return true }
            last = bar
            Thread.sleep(forTimeInterval: 0.25)
        }
        return false
    }

    private func topBarPixels() -> Data? {
        guard let image = XCUIScreen.main.screenshot().image.cgImage else { return nil }
        let height = CGFloat(image.height)
        let strip = CGRect(x: 0, y: height * 0.07, width: CGFloat(image.width), height: height * 0.07)
        return image.cropping(to: strip).flatMap { UIImage(cgImage: $0).pngData() }
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) != nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) != nil
    }

    private func waitGone(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) == nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) == nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
