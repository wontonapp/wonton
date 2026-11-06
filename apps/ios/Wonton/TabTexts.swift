import Foundation

/// 壳负责原生控件的文字；清单标题由核心选择语言。
enum TabText {
    static func choose(_ chinese: String, _ english: String) -> String {
        Locale.preferredLanguages.first?.hasPrefix("zh") == true ? chinese : english
    }

    static var chat: String { choose("聊天", "Chats") }
    static var apps: String { choose("应用", "Apps") }
    static var settings: String { choose("设置", "Settings") }
    static var bottomBar: String { choose("底栏", "Tab bar") }
    static var showLabels: String { choose("显示文字", "Show labels") }
    static var addMore: String { choose("添加更多", "Add more") }
    static var addToBar: String { choose("添加到底栏", "Add to tab bar") }
    static var pin: String { choose("固定到底栏", "Pin to tab bar") }
    static var unpin: String { choose("从底栏移除", "Remove from tab bar") }
    static var replace: String { choose("底栏已满，换掉哪一项？", "Tab bar full. Replace which tab?") }
    static var back: String { choose("返回", "Back") }
    static var close: String { choose("关闭", "Close") }
    static var retry: String { choose("重试", "Retry") }
    static var details: String { choose("查看详情", "View details") }
    static var open: String { choose("打开", "Open") }
    static var pinToBar: String { choose("放到底栏", "Pin to tab bar") }
    static var fullscreen: String { choose("全屏打开", "Open fullscreen") }
    static var alwaysFullscreen: String { choose("总是全屏", "Always fullscreen") }
    static var reload: String { choose("重新加载", "Reload") }
    static var more: String { choose("更多", "More") }
    static var starting: String { choose("正在启动…", "Starting…") }
    static var loading: String { choose("正在加载", "Loading") }
    static var upgrading: String { choose("正在更新…", "Updating…") }
    static var uninstalling: String { choose("正在卸载…", "Uninstalling…") }
    static var pageRemoved: String { choose("新版本不再提供这个页面", "This page is no longer available") }
    static var pageFailed: String { choose("页面打不开", "Could not open this page") }
    static var unknown: String { choose("无法确认应用的状态", "The app’s status is unknown") }
    static var configurationFailed: String {
        choose("底栏设置读不了，已使用默认底栏", "Could not read tab settings. Using the default tabs.")
    }
    static var selectChat: String { choose("选一个聊天放入草稿", "Choose a chat for your draft") }
    static var done: String { choose("完成", "Done") }
    static var start: String { choose("启动", "Start") }
    static var failed: String { choose("出了问题", "has a problem") }

    static func stopped(_ title: String) -> String {
        choose("\(title) 已停止", "\(title) is stopped")
    }

    static func uninstalled(_ title: String) -> String {
        choose("\(title) 已卸载，已从底栏移除", "\(title) was uninstalled and removed from the tab bar")
    }
}
