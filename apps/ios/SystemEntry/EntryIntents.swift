import AppIntents
import Foundation

/// 控制按钮的打开目标（ADR-0100 决策 5）：问 Bot 固定默认 Agent 的私聊，插件从快照的 plugins 里选。
/// 这个类型和下面的意图同时编进 App 和小组件扩展（OpenIntent 的要求）；快照从 App Group 读，不依赖核心。
/// AppIntents 的元数据提取只要字面量，title、参数标题和类型名用英文字面量当键（照 AskAgentIntent 的写法），
/// 译文在两边的默认 String Catalog 里。
struct EntryDestination: AppEntity, Sendable {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Plugin"
    static let defaultQuery = EntryDestinationQuery()

    let id: String
    let title: String

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(title)") }

    /// 没选插件时打开应用页。
    static var apps: Self {
        Self(id: EntryLink.apps.absoluteString, title: String(localized: .SystemEntry.systemEntryApps))
    }

    static var agent: Self {
        Self(id: EntryLink.agent.absoluteString, title: String(localized: .SystemEntry.systemEntryAskBot))
    }
}

struct EntryDestinationQuery: EntityStringQuery {
    func entities(for identifiers: [String]) async throws -> [EntryDestination] {
        let available = try [EntryDestination.agent, .apps] + plugins()
        return identifiers.compactMap { id in available.first { $0.id == id } }
    }

    func suggestedEntities() async throws -> [EntryDestination] { try plugins() }

    func entities(matching string: String) async throws -> [EntryDestination] {
        try plugins().filter { $0.title.localizedStandardContains(string) }
    }

    private func plugins() throws -> [EntryDestination] {
        let state = EntrySnapshotStore.read()
        if case .unreadable = state { throw EntryIntentFailure.launchFailed }
        guard let snapshot = state.signedIn else { return [] }
        return snapshot.plugins.map {
            EntryDestination(id: EntryLink.plugin($0.pluginId).absoluteString, title: $0.title)
        }
    }
}

/// 控制按钮和小组件都把人送进 App 的某一处；OpenIntent 要同时编进两个 target，由 App 进程执行。
struct OpenSystemEntryIntent: OpenIntent {
    static let title: LocalizedStringResource = "Open Wonton"

    @Parameter(title: LocalizedStringResource("Target"))
    var target: EntryDestination

    init() {}

    init(target: EntryDestination) { self.target = target }

    @MainActor
    func perform() async throws -> some IntentResult {
        #if WONTON_APP
        guard let url = URL(string: target.id) else { throw EntryIntentFailure.launchFailed }
        try AppNavigation.requireShared().receive(url)
        #else
        // 系统只在 App 进程里跑 OpenIntent；扩展里的这份不该被执行。
        throw EntryIntentFailure.launchFailed
        #endif
        return .result()
    }
}

enum EntryIntentFailure: LocalizedError {
    /// 核心没起来或快照读不了：如实报错，不装作成功。
    case launchFailed

    var errorDescription: String? { String(localized: .SystemEntry.systemEntryLaunchFailed) }
}
