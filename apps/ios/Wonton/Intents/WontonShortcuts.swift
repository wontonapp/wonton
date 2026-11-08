import AppIntents

struct WontonShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AskAgentIntent(),
            phrases: ["Ask a bot with \(.applicationName)", "Ask a bot in \(.applicationName)"],
            shortTitle: "Ask a Bot",
            systemImageName: "bubble.left.and.bubble.right"
        )
        AppShortcut(
            intent: NewChatIntent(),
            phrases: ["New chat in \(.applicationName)", "Start a chat with \(.applicationName)"],
            shortTitle: "New Chat",
            systemImageName: "square.and.pencil"
        )
        AppShortcut(
            intent: OpenPageIntent(),
            phrases: ["Open \(\.$page) with \(.applicationName)", "Open \(\.$page) in \(.applicationName)"],
            shortTitle: "Open Page",
            systemImageName: "square.grid.2x2"
        )
    }
}
