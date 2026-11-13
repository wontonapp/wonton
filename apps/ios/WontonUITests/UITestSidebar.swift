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
                if row.exists && row.isHittable { return row }
            }
            if let containing, opened {
                let row = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", containing)).firstMatch
                if row.exists && row.isHittable { return row }
            }
            if !opened {
                let open = app.buttons.matching(button).firstMatch
                if open.exists && open.isHittable {
                    open.tap()
                    opened = true
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
