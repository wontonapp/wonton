import SwiftUI
import WidgetKit

/// 快照的 timeline：系统预算刷新（约 15–60 分钟一次）时重读文件；App 每写一份会调 reloadTimelines（ADR-0100 决策 3）。
struct SnapshotEntry: TimelineEntry {
    let date: Date
    let state: EntrySnapshotState
}

struct SnapshotProvider: TimelineProvider {
    func placeholder(in context: Context) -> SnapshotEntry {
        SnapshotEntry(date: .now, state: EntrySnapshotStore.read())
    }

    func getSnapshot(in context: Context, completion: @escaping (SnapshotEntry) -> Void) {
        completion(SnapshotEntry(date: .now, state: EntrySnapshotStore.read()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<SnapshotEntry>) -> Void) {
        let now = Date.now
        completion(Timeline(
            entries: [SnapshotEntry(date: now, state: EntrySnapshotStore.read())],
            policy: .after(now.addingTimeInterval(30 * 60))
        ))
    }
}

/// 最近会话/未读（ADR-0100）：中号放最近几个会话，锁屏 inline 只放总未读数。
struct RecentChatsWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "WontonRecentChats", provider: SnapshotProvider()) { entry in
            EntryWidgetContent(kind: .recent, state: entry.state)
        }
        .configurationDisplayName(.SystemEntry.systemEntryRecentChats)
        .description(.SystemEntry.systemEntryRecentChatsDescription)
        .supportedFamilies([.systemMedium, .accessoryInline])
    }
}

/// Bot 状态（ADR-0100）：小号放默认 Agent 的名字和在做什么，锁屏 rectangular 只放状态和未读数。
struct BotStatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "WontonBotStatus", provider: SnapshotProvider()) { entry in
            EntryWidgetContent(kind: .status, state: entry.state)
        }
        .configurationDisplayName(.SystemEntry.systemEntryBotStatus)
        .description(.SystemEntry.systemEntryBotStatusDescription)
        .supportedFamilies([.systemSmall, .accessoryRectangular])
    }
}

/// 问 Bot（ADR-0100）：小号和锁屏圆形，点了进和默认 Agent 的私聊。
struct AskBotWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "WontonAskBot", provider: SnapshotProvider()) { entry in
            EntryWidgetContent(kind: .ask, state: entry.state)
        }
        .configurationDisplayName(.SystemEntry.systemEntryAskBot)
        .description(.SystemEntry.systemEntryAskBotDescription)
        .supportedFamilies([.systemSmall, .accessoryCircular])
    }
}

private struct EntryWidgetContent: View {
    let kind: EntryWidgetView.Kind
    let state: EntrySnapshotState

    @Environment(\.widgetFamily) private var family

    var body: some View {
        EntryWidgetView(kind: kind, state: state, family: Self.entryFamily(family))
    }

    private static func entryFamily(_ family: WidgetFamily) -> EntryWidgetView.Family {
        switch family {
        case .systemSmall: .systemSmall
        case .accessoryInline: .accessoryInline
        case .accessoryRectangular: .accessoryRectangular
        case .accessoryCircular: .accessoryCircular
        default: .systemMedium
        }
    }
}

@main
struct WontonWidgets: WidgetBundle {
    var body: some Widget {
        RecentChatsWidget()
        BotStatusWidget()
        AskBotWidget()
        if #available(iOS 18.0, *) {
            AskBotControl()
            OpenPluginControl()
        }
    }
}
