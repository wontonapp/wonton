import AppIntents

struct WontonShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AskAgentIntent(),
            phrases: ["用\(.applicationName)问 Bot", "在\(.applicationName)里问 Bot"],
            shortTitle: "问 Bot",
            systemImageName: "bubble.left.and.bubble.right"
        )
        AppShortcut(
            intent: NewChatIntent(),
            phrases: ["在\(.applicationName)里新建会话", "用\(.applicationName)发起私聊"],
            shortTitle: "新建会话",
            systemImageName: "square.and.pencil"
        )
        AppShortcut(
            intent: OpenPageIntent(),
            phrases: ["用\(.applicationName)打开 \(\.$page)", "在\(.applicationName)里打开 \(\.$page)"],
            shortTitle: "打开页面",
            systemImageName: "square.grid.2x2"
        )
    }
}
