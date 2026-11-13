import SwiftUI

/// 系统入口小组件的渲染（ADR-0100）：只读快照画界面，不跑核心。App 和扩展共用（截图测试也画它），
/// 颜色用快照里的令牌按系统明暗取一套；锁屏 accessory 由系统提供底色和着色，改用系统原色保证可读。
struct EntryWidgetView: View {
    enum Kind { case recent, status, ask }
    enum Family { case systemMedium, systemSmall, accessoryInline, accessoryRectangular, accessoryCircular }

    let kind: Kind
    let state: EntrySnapshotState
    let family: Family

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        switch state {
        case let .ready(snapshot):
            if let notice = Self.notice(snapshot, kind: kind) {
                NoticeView(text: notice, url: fallbackURL)
            } else {
                content(snapshot)
            }
        case .missing:
            NoticeView(text: String(localized: .SystemEntry.systemEntryOpenToStart), url: fallbackURL)
        case .unreadable:
            NoticeView(text: String(localized: .SystemEntry.systemEntryOpenToRefresh), url: fallbackURL)
        }
    }

    private var fallbackURL: URL {
        kind == .recent ? EntryLink.chat : EntryLink.agent
    }

    /// 没有内容可画时的一句话（措辞同 Android 的 accountNotice）；已登录、有默认 Agent 时为空。
    private static func notice(_ snapshot: EntrySnapshot, kind: Kind) -> String? {
        switch snapshot.account {
        case .signedIn:
            if kind == .status, snapshot.defaultAgent == nil {
                return String(localized: .SystemEntry.systemEntryChooseAgent)
            }
            return nil
        case .notPaired:
            return String(localized: .SystemEntry.systemEntryOpenToPair)
        case .signedOut:
            return String(localized: .SystemEntry.systemEntryOpenToSignIn)
        case .switching:
            return String(localized: .SystemEntry.systemEntryOpenToRefresh)
        }
    }

    @ViewBuilder
    private func content(_ snapshot: EntrySnapshot) -> some View {
        let palette = scheme == .dark ? snapshot.dark : snapshot.light
        familyContent(snapshot, palette: palette)
            .widgetURL(fallbackURL)
            .containerBackground(for: .widget) {
                family == .systemMedium || family == .systemSmall ? palette.surface.color : Color.clear
            }
    }

    @ViewBuilder
    private func familyContent(_ snapshot: EntrySnapshot, palette: EntrySnapshot.Palette) -> some View {
        switch (kind, family) {
        case (.recent, .accessoryInline):
            InlineUnread(snapshot: snapshot)
        case (.status, .accessoryRectangular):
            RectangularStatus(snapshot: snapshot)
        case (.ask, .accessoryCircular):
            AskRound()
        case (.recent, .systemMedium):
            RecentRows(snapshot: snapshot, palette: palette)
        case (.status, .systemSmall):
            StatusCard(snapshot: snapshot, palette: palette)
        case (.ask, .systemSmall):
            AskCard(palette: palette)
        default:
            // supportedFamilies 之外的组合不出现。
            RecentRows(snapshot: snapshot, palette: palette)
        }
    }
}

/// 没配对、登录失效、刚换服务器、快照读不了时的一句话，点了开 App，由 App 接着说。
private struct NoticeView: View {
    let text: String
    let url: URL

    var body: some View {
        Text(verbatim: text)
            .font(.system(size: 13))
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .lineLimit(3)
            .padding(12)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .widgetURL(url)
            .containerBackground(for: .widget) { Color.clear }
    }
}

private struct RecentRows: View {
    let snapshot: EntrySnapshot
    let palette: EntrySnapshot.Palette

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if snapshot.rooms.isEmpty {
                Text(String(localized: .SystemEntry.systemEntryNoChats))
                    .font(.system(size: 12))
                    .foregroundStyle(palette.textSecondary.color)
            } else {
                ForEach(Array(snapshot.rooms.prefix(3).enumerated()), id: \.offset) { _, room in
                    RoomRow(room: room, palette: palette)
                }
            }
            Spacer(minLength: 0)
            footer
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    /// “上次更新于”和相对时间各占一段：相对时间由系统走时，不随刷新停摆（ADR-0100 决策 3）。
    private var footer: some View {
        (Text(String(localized: .SystemEntry.systemEntryUpdatedLabel))
            + Text(verbatim: " ")
            + Text(snapshot.updatedAt, style: .relative))
            .font(.system(size: 11))
            .foregroundStyle(palette.textSecondary.color)
            .lineLimit(1)
    }
}

private struct RoomRow: View {
    let room: EntrySnapshot.Room
    let palette: EntrySnapshot.Palette

    var body: some View {
        Link(destination: EntryLink.room(room.roomId)) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: room.name)
                        .font(.system(size: 14, weight: room.unreadCount > 0 ? .semibold : .medium))
                        .foregroundStyle(palette.textPrimary.color)
                        .lineLimit(1)
                    if let preview {
                        Text(verbatim: preview)
                            .font(.system(size: 12))
                            .foregroundStyle(palette.textSecondary.color)
                            .lineLimit(1)
                            .privacySensitive()
                    }
                }
                Spacer(minLength: 0)
                if room.unreadCount > 0 { UnreadBadge(count: room.unreadCount, palette: palette) }
            }
            .contentShape(Rectangle())
        }
        .frame(height: 36)
    }

    /// 预览正文原样，图片、文件等写占位（同聊天列表，措辞照 message.summary*）；旁聊给标题和状态。
    private var preview: String? {
        guard let preview = room.preview else { return nil }
        let line = Self.line(preview.body)
        guard !line.isEmpty else { return nil }
        return preview.isOwn ? String(localized: .SystemEntry.systemEntryOwnPreview(line)) : line
    }

    private static func line(_ body: EntrySnapshot.PreviewBody) -> String {
        switch body.type {
        case "text":
            return body.text ?? ""
        case "files":
            return String(localized: .SystemEntry.systemEntrySummaryFiles)
        case "image":
            return String(localized: .SystemEntry.systemEntrySummaryImage)
        case "undecryptable":
            return String(localized: .SystemEntry.systemEntrySummaryUndecryptable)
        case "redacted":
            return String(localized: .SystemEntry.systemEntrySummaryRedacted)
        case "side_chat":
            guard let title = body.title else { return "" }
            // 没有支持声明就不会有结果行（ADR-0093）：只给标题。
            if body.state == "untracked" { return title }
            return String(localized: .SystemEntry.systemEntrySideChatResult(title, sideChatState(body.state)))
        default:
            // 事件类型原文（如 m.sticker），不翻译。
            return "[\(body.kind ?? body.type)]"
        }
    }

    private static func sideChatState(_ state: String?) -> String {
        switch state {
        case "open":
            return String(localized: .SystemEntry.systemEntrySideChatOpen)
        case "done":
            return String(localized: .SystemEntry.systemEntrySideChatDone)
        case "failed":
            return String(localized: .SystemEntry.systemEntrySideChatFailed)
        case "ended":
            return String(localized: .SystemEntry.systemEntrySideChatEnded)
        case "result_missing":
            return String(localized: .SystemEntry.systemEntrySideChatResultMissing)
        default:
            return String(localized: .SystemEntry.systemEntrySideChatOther)
        }
    }
}

private struct StatusCard: View {
    let snapshot: EntrySnapshot
    let palette: EntrySnapshot.Palette

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image("EntryAgent", bundle: nil)
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 20, height: 20)
                    .foregroundStyle(palette.accentInk.color)
                Text(verbatim: snapshot.defaultAgent?.name ?? "")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(palette.textPrimary.color)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if let unread = snapshot.defaultAgent?.unreadCount, unread > 0 {
                    UnreadBadge(count: unread, palette: palette)
                }
            }
            statusLine
                .font(.system(size: 11))
                .foregroundStyle(palette.textSecondary.color)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .padding(.horizontal, 12)
    }

    private var statusLine: Text {
        let activity = Self.activityText(snapshot.defaultAgent?.activity)
        return Text(verbatim: activity) + Text(verbatim: " · ") + Text(snapshot.updatedAt, style: .relative)
    }

    /// Bot 此刻在做什么（措辞照 bot.state.*）；没取到如实写状态未知。
    private static func activityText(_ activity: String?) -> String {
        switch activity {
        case "idle":
            return String(localized: .SystemEntry.systemEntryStateIdle)
        case "working":
            return String(localized: .SystemEntry.systemEntryStateWorking)
        case "waiting":
            return String(localized: .SystemEntry.systemEntryStateWaiting)
        case "offline":
            return String(localized: .SystemEntry.systemEntryStateOffline)
        default:
            return String(localized: .SystemEntry.systemEntryStatusUnknown)
        }
    }
}

/// 锁屏的 Bot 状态：只放未读数和状态，不放人名（ADR-0100 决策 8）。
private struct RectangularStatus: View {
    let snapshot: EntrySnapshot

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: Self.activityText(snapshot.defaultAgent?.activity))
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
            Text(String(localized: .SystemEntry.systemEntryUnreadCount(
                Int(clamping: snapshot.defaultAgent?.unreadCount ?? 0)
            )))
            .font(.system(size: 12))
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
    }

    private static func activityText(_ activity: String?) -> String {
        switch activity {
        case "idle":
            return String(localized: .SystemEntry.systemEntryStateIdle)
        case "working":
            return String(localized: .SystemEntry.systemEntryStateWorking)
        case "waiting":
            return String(localized: .SystemEntry.systemEntryStateWaiting)
        case "offline":
            return String(localized: .SystemEntry.systemEntryStateOffline)
        default:
            return String(localized: .SystemEntry.systemEntryStatusUnknown)
        }
    }
}

/// 锁屏的最近聊天：只显示总未读数（ADR-0100 决策 8）。
private struct InlineUnread: View {
    let snapshot: EntrySnapshot

    var body: some View {
        Text(String(localized: .SystemEntry.systemEntryUnreadCount(Int(clamping: snapshot.unreadTotal))))
            .font(.system(size: 14, weight: .medium))
            .foregroundStyle(.primary)
            .lineLimit(1)
    }
}

private struct AskCard: View {
    let palette: EntrySnapshot.Palette

    var body: some View {
        VStack(spacing: 4) {
            Image("EntryAgent", bundle: nil)
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 28, height: 28)
                .foregroundStyle(palette.accentInk.color)
            Text(String(localized: .SystemEntry.systemEntryAskBot))
                .font(.system(size: 11))
                .foregroundStyle(palette.textPrimary.color)
                .lineLimit(1)
        }
    }
}

private struct AskRound: View {
    var body: some View {
        VStack(spacing: 2) {
            Image("EntryAgent", bundle: nil)
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 22, height: 22)
                .foregroundStyle(.primary)
            Text(String(localized: .SystemEntry.systemEntryAskBot))
                .font(.system(size: 9))
                .foregroundStyle(.primary)
                .lineLimit(1)
        }
    }
}

/// 未读数的小圆标：重点色填充（ADR-0026），数字按十进制原样。
private struct UnreadBadge: View {
    let count: UInt64
    let palette: EntrySnapshot.Palette

    var body: some View {
        Text(verbatim: String(count))
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(palette.accentInkOnFill.color)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(palette.accentFill.color, in: Capsule())
            .lineLimit(1)
    }
}
