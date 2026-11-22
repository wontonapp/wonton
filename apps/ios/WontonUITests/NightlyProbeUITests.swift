import XCTest

/// 临时：查房间里顶栏控件、整页里引导按钮，在无障碍里排在滚动视图/网页的前面还是后面，以及点不点得到。
/// 层排反了按钮就点不到（夜跑 iOS 界面任务里的那几条失败）。只打印，不做断言。
final class NightlyProbeUITests: XCTestCase {
    private var app: XCUIApplication!
    /// 探针要真起 App、点进房间，最坏等 40+40 秒；给足允许时间，别被默认 90 秒砍掉。
    override func setUp() {
        super.setUp()
        executionTimeAllowance = 180
    }

    /// 每 0.7 秒重采一次，直到这几个名字都在（网页加载完会换一次快照，读旧元素会报错）。
    private func order(_ names: [String]) -> String {
        var text = ""
        for _ in 0..<6 {
            let all = app.descendants(matching: .any).allElementsBoundByIndex
            var lines: [String] = []
            var missing = false
            for name in names {
                var index = -1
                var hittable = false
                for (i, element) in all.enumerated() {
                    let byType = (name == "CollectionView" && element.elementType == .collectionView)
                        || (name == "WebView" && element.elementType == .webView)
                    if byType || element.identifier == name || element.label == name {
                        index = i
                        hittable = element.isHittable
                        break
                    }
                }
                if index < 0 { missing = true }
                lines.append("\(name)=排第 \(index)（共 \(all.count) 个），hittable=\(hittable)")
            }
            text = lines.joined(separator: "；")
            if !missing { return text }
            Thread.sleep(forTimeInterval: 0.7)
        }
        return text
    }

    func testBarLayerOrder() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launch()
        let room = app.buttons.matching(NSPredicate(format: "identifier == %@", "chatRoom.!ui-fake-call:home.example"))
        XCTAssertTrue(room.firstMatch.waitForExistence(timeout: 20))
        room.firstMatch.tap()
        XCTAssertTrue(app.textViews["Message"].waitForExistence(timeout: 10))
        Thread.sleep(forTimeInterval: 3)
        // 像人一样先往上拖一下列表，再看：滚动过的列表最容易压在栏上面。
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 1")).firstMatch
            .swipeUp()
        Thread.sleep(forTimeInterval: 1)
        XCTContext.runActivity(named: "探针 顶栏层序 \(order(["CollectionView", "roomCallActive", "Back", "Message"]))") { _ in }
    }

    func testHintLayerOrder() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_PAGE_EXIT_HINT"] = "1"
        app.launchEnvironment["WONTON_UI_URL_AS_APP_PAGE"] = "1"
        app.launch()
        let sidebar = app.buttons.matching(NSPredicate(format: "label IN %@", ["Sidebar", "侧边栏", "側邊欄", "サイドバー"]))
        XCTAssertTrue(sidebar.firstMatch.waitForExistence(timeout: 40))
        openCorner()
        Thread.sleep(forTimeInterval: 2)
        XCTContext.runActivity(named: "探针 引导层序 第一次 \(order(["pageExitHintDismiss", "WebView"]))") { _ in }

        // 不点“知道了”，左边缘右滑回聊天，再打开一次。
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
        XCTAssertTrue(sidebar.firstMatch.waitForExistence(timeout: 40))
        openCorner()
        Thread.sleep(forTimeInterval: 3)
        XCTContext.runActivity(named: "探针 引导层序 第二次 \(order(["pageExitHintDismiss", "WebView"]))") { _ in }
    }

    private func openCorner() {
        app.buttons.matching(NSPredicate(format: "label IN %@", ["Sidebar", "侧边栏", "側邊欄", "サイドバー"])).firstMatch.tap()
        app.buttons.matching(NSPredicate(format: "label == %@", "All")).firstMatch.tap()
        app.staticTexts.matching(NSPredicate(format: "label == %@", "Corner")).firstMatch.tap()
    }
}
