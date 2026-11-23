import XCTest

/// 底栏没了（ADR-0095）：页面入口都在侧边栏。给各测试类用的导航助手。
enum UITestSidebar {
    /// 根页面左上角的侧边栏按钮。
    static var button: NSPredicate {
        NSPredicate(format: "label IN %@", ["Sidebar", "侧边栏", "側邊欄", "サイドバー"])
    }

    /// 主界面到了：侧边栏按钮存在（配对后只有聊天一个入口，没有别的标签可等）。
    @discardableResult
    static func waitForMain(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if app.buttons.matching(button).firstMatch.exists { return true }
            // app.wait(for: .runningForeground) 在前台时立刻返回，不能拿来当间隔。
            Thread.sleep(forTimeInterval: 0.5)
        }
        return false
    }

    /// 聊天列表到了。配对页也有侧边栏按钮，不能只看它；交接/备份页是盖在列表上的层，
    /// 下面的列表元素照样能查到，它们在场时也不算。认列表自己的东西：空列表的提示、新聊天按钮或已有的 Hermes 行。
    static func chatListVisible(_ app: XCUIApplication) -> Bool {
        if app.staticTexts["Continue on your other device"].firstMatch.exists { return false }
        if app.scrollViews["chat-backup"].exists { return false }
        if app.staticTexts["No chats yet"].firstMatch.exists { return true }
        let newChat = app.buttons.matching(NSPredicate(format: "label IN %@", ["New chat", "New Chat"])).firstMatch
        if newChat.exists { return true }
        return app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Hermes")).firstMatch.exists
    }

    @discardableResult
    static func waitForChatList(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if chatListVisible(app) { return true }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return false
    }

    /// 开侧边栏，返回 labels 里第一个能点的行；调用方负责点。开不了、找不到返回 nil，抽屉留在原样。
    /// `containing` 按标签里含的字找（应用名会带上中文名），先开抽屉再找，不点到聊天列表里同名的房间。
    static func row(
        _ app: XCUIApplication, labels: [String], containing: String? = nil, timeout: TimeInterval = 8
    ) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        var opened = false
        while Date() < deadline {
            for label in labels {
                let row = app.buttons[label].firstMatch
                // 抽屉动画中途行会短暂落在屏幕外（负 x），只要还在树里就算找到，点的时候按坐标点。
                if row.exists { return row }
            }
            if let containing, opened {
                let row = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", containing)).firstMatch
                if row.exists { return row }
            }
            if !opened {
                let open = app.buttons.matching(button).firstMatch
                if open.exists {
                    open.tap()
                    opened = true
                    Thread.sleep(forTimeInterval: 0.8)   // 抽屉滑出来的动画，落位以后再找行
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    /// “全部应用”那一行只在还有没加进“我的应用”的应用时才有（插件列表没到时也没有）；“我的应用”标题旁的
    /// “全部”一直在，找不到行时用它。
    static func allApps(_ app: XCUIApplication, timeout: TimeInterval = 8) -> XCUIElement? {
        row(
            app,
            labels: ["All Apps", "全部应用", "全部應用程式", "すべてのアプリ", "All", "全部", "すべて"],
            timeout: timeout
        )
    }
}
