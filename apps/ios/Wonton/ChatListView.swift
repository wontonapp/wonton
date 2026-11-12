import SwiftUI
import WontonClient

struct ChatListView: View {
    @Bindable var model: ChatModel
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors
    @Environment(TabsModel.self) private var tabs
    @Environment(\.chatPull) private var pull
    @State private var confirmation: PluginConfirmation?

    var body: some View {
        NavigationStack(path: $model.path) {
            ScrollView {
                LazyVStack(spacing: 0) {
                    notices
                    if model.pendingDraft != nil {
                        Text(TabText.selectChat)
                            .wontonText(WontonTheme.typography.supporting)
                            .padding(WontonTheme.spacing.pageInset)
                    }
                    let installing = model.viewModel.installingBots
                    ForEach(Array(installing.enumerated()), id: \.element.pluginId) { index, row in
                        if index > 0 { separator }
                        ChatInstallRowView(
                            row: row,
                            plugins: plugins,
                            retry: { retry(row) },
                            openPage: { tabs.select(.apps) }
                        )
                    }
                    ForEach(Array(model.viewModel.rooms.enumerated()), id: \.element.roomId) { index, row in
                        if index > 0 || !installing.isEmpty { separator }
                        if row.membership == .invited {
                            InviteRow(row: row, model: model)
                        } else {
                            NavigationLink(value: ChatRoute.room(row.roomId)) {
                                RoomRowView(row: row, model: model, plugins: plugins)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .background { if let pull { ChatPullDetector(pull: pull) } }
            }
            .overlay {
                if isEmpty { NoChats() }
            }
            .safeAreaInset(edge: .bottom, alignment: .trailing, spacing: 0) {
                newChatButton
                    .padding(.trailing, WontonTheme.spacing.s16)
                    .padding(.bottom, WontonTheme.spacing.s12)
            }
            .wontonRootScreen(TabText.chat, hasActions: false) { EmptyView() }
            .fullScreenCover(item: $confirmation) { InstallConfirmView(model: plugins, confirmation: $0) }
            .environment(\.goToInstalledApps, { tabs.select(.apps) })
            .navigationDestination(for: ChatRoute.self) { route in
                switch route {
                case let .room(roomId):
                    // 提示条把路径直接换成另一个房间时，SwiftUI 会复用这个视图，@State 里还是旧房间的时间线。
                    ChatRoomView(model: model, plugins: plugins, roomId: roomId, title: model.title(of: roomId))
                        .id(roomId)
                case .chatBackup:
                    ChatBackupView(model: model)
                case .startChat:
                    StartChatView(model: model)
                }
            }
        }
    }

    private var separator: some View {
        WontonSeparator().padding(.leading, WontonTheme.size.avatar + WontonTheme.spacing.s12)
    }

    /// 重试不进核心的执行路径：重新预览，主人在确认页上再点一次（ADR-0067）。
    private func retry(_ row: BotInstallRow) {
        switch row.operation {
        case .upgrade: confirmation = .upgrade(id: row.pluginId)
        case .install, .uninstall:
            guard let source = row.source else { return }
            confirmation = .install(source: source)
        }
    }

    private var newChatButton: some View {
        NavigationLink(value: ChatRoute.startChat) {
            WontonIcon.add.view()
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: WontonTheme.size.floatingActionButton, height: WontonTheme.size.floatingActionButton)
                .background(.ultraThinMaterial, in: Circle())
                .overlay { Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline) }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(.chatNewChat))
    }

    @ViewBuilder private var notices: some View {
        let status = ChatStatus(model.viewModel)
        let reminder = model.viewModel.encryption.backupReminder?.text
        if status != nil || reminder != nil {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                status.map(ChatStatusView.init)
                if let reminder {
                    WontonGroup {
                        if model.viewModel.encryption.backupReminder == .continueOnOtherDevice {
                            // 交接还没完成，主人先进了列表：点了回到交接页（ADR-0087）。
                            Button { model.keyHandoffDeferred = false } label: {
                                WontonRow(title: reminder, showsForward: true)
                            }
                            .buttonStyle(.plain)
                        } else {
                            NavigationLink(value: ChatRoute.chatBackup) { WontonRow(title: reminder, showsForward: true) }
                                .buttonStyle(.plain)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    /// 同步好之前列表是空的不代表没有聊天。
    private var isEmpty: Bool {
        guard model.viewModel.rooms.isEmpty, model.viewModel.installingBots.isEmpty,
              case .signedIn = model.viewModel.account, case .running = model.viewModel.sync else { return false }
        return true
    }

}

/// 聊天操作的结果。只在底栏之上挂一份：挂在各页的话，后台页的那份先弹，弹不出来，用户在别的页就看不到。
struct ChatDialogs: ViewModifier {
    let model: ChatModel

    func body(content: Content) -> some View {
        content
            .wontonDialog(problem: model.rejection, isPresented: isShowingRejection)
            .wontonDialog(
                problem: model.viewModel.problem?.problem,
                isPresented: isShowingProblem,
                onDismiss: { model.send(.dismissChatProblem) }
            )
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(get: { model.rejection != nil }, set: { if !$0 { model.rejection = nil } })
    }

    private var isShowingProblem: Binding<Bool> {
        Binding(
            get: { model.viewModel.problem != nil },
            set: { if !$0 { model.send(.dismissChatProblem) } }
        )
    }
}

private enum ChatStatus {
    case signedOut
    case progress(String)
    case failed(Problem)

    init?(_ viewModel: ChatViewModel) {
        switch viewModel.account {
        case .signedOut:
            self = .signedOut
        case .signingIn:
            self = .progress(String(localized: .chatLoggingIn))
        case .reconnecting:
            self = .progress(String(localized: .chatStatusReconnecting))
        case let .failed(reason, message):
            // 连不上由标题栏说，连上后自动重来。
            if reason == .transportUnavailable { return nil }
            self = .failed(reason.problem(detail: message))
        case .signedIn:
            guard case let .failed(reason, message) = viewModel.sync, reason != .transportUnavailable else {
                return nil
            }
            self = .failed(reason.problem(detail: message))
        }
    }
}

private struct ChatStatusView: View {
    let status: ChatStatus
    @Environment(\.wontonColors) private var colors

    init(_ status: ChatStatus) {
        self.status = status
    }

    var body: some View {
        switch status {
        case .signedOut:
            WontonNotice(icon: .connection, title: String(localized: .chatNotSignedIn), isError: false)
        case let .progress(text):
            Text(text)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case let .failed(problem):
            ProblemRow(problem: problem)
        }
    }
}

private struct VisitedRoom: Hashable {
    let roomId: String
}

/// 从 `tab` 这一页打开的私聊压进这一页的导航路径，返回回到点的地方（比如插件详情）。
/// `navigationDestination(item:)` 放在根上会先弹掉已经压进去的页面，所以走路径。
struct VisitingRoomModifier: ViewModifier {
    let model: ChatModel
    let plugins: PluginsModel
    let tab: AppTab
    @Binding var path: NavigationPath
    @State private var depth = 0

    func body(content: Content) -> some View {
        content
            .navigationDestination(for: VisitedRoom.self) { room in
                ChatRoomView(model: model, plugins: plugins, roomId: room.roomId, title: model.title(of: room.roomId))
                    .id(room.roomId)
            }
            .onChange(of: model.shownRoom(in: tab)) { _, roomId in
                if let roomId {
                    depth = path.count
                    path.append(VisitedRoom(roomId: roomId))
                } else if path.count > depth {
                    path.removeLast(path.count - depth)
                }
            }
            .onChange(of: path.count) { _, count in
                if count <= depth, model.shownRoom(in: tab) != nil { model.visiting = nil }
            }
    }
}

private struct NoChats: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(.chatEmpty)
            .wontonText(WontonTheme.typography.itemTitle)
            .foregroundStyle(Color(colors.textPrimary))
    }
}

private struct RoomRowView: View {
    let row: RoomRow
    let model: ChatModel
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            RoomAvatar(row: row, model: model, plugins: plugins)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        Text(row.name)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                            .lineLimit(1)
                        if row.isEncrypted {
                            WontonIcon.lockFill.view(size: WontonTheme.size.iconTiny)
                                .foregroundStyle(Color(colors.textSecondary))
                                .accessibilityLabel(Text(.chatEncrypted))
                        }
                    }
                    Spacer(minLength: 0)
                    if let preview = row.preview {
                        Text(RoomTime.text(preview.timestampMs))
                            .wontonText(WontonTheme.typography.caption)
                            .monospacedDigit()
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                HStack(spacing: WontonTheme.spacing.s8) {
                    if let preview = row.preview {
                        previewLine(preview)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                    if row.unreadCount > 0 {
                        Text("\(row.unreadCount)")
                            .wontonText(WontonTheme.typography.caption)
                            .monospacedDigit()
                            .foregroundStyle(Color(colors.accentInkOnFill))
                            .padding(.horizontal, WontonTheme.spacing.s8)
                            .padding(.vertical, WontonTheme.spacing.s4)
                            .accentFill(Capsule())
                            .accessibilityLabel(Text(.chatUnread(count: Int(row.unreadCount))))
                    }
                }
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .contentShape(Rectangle())
    }

    /// 预览正文原样显示；自己发的在前面标一句“我：”。
    private func previewLine(_ preview: RoomPreview) -> Text {
        preview.isOwn ? Text(.chatPreviewOwn(preview.body.line)) : Text(preview.body.line)
    }
}

/// Bot 的私聊行用 Bot 头像（状态头像优先，ADR-0044），其他装了应用的用应用图标，其余人用名字的第一个字。
struct RoomAvatar: View {
    let row: RoomRow
    let model: ChatModel
    let plugins: PluginsModel
    var size = WontonTheme.size.avatar
    @Environment(\.wontonColors) private var colors

    private var plugin: InstalledPlugin? {
        guard let bot = row.directUserId else { return nil }
        return plugins.viewModel.installed?.plugins.first { $0.botUserId == bot }
    }

    var body: some View {
        if let bot = row.directUserId.flatMap({ id in model.viewModel.bots.first { $0.userId == id } }) {
            BotAvatarView(bot: bot, pluginIcon: plugin?.icon, core: plugins.core, model: plugins, size: size)
        } else if let plugin {
            PluginIconView(icon: plugin.icon, size: size, model: plugins)
        } else {
            Text(row.name.prefix(1).uppercased())
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: size, height: size)
                .background(Circle().fill(Color(colors.surfaceSubtle)))
                .accessibilityHidden(true)
        }
    }
}

/// 列表里的时间：今天的写几点，昨天写“昨天”，一周内写星期，再早写日期。
private enum RoomTime {
    static func text(_ timestampMs: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestampMs) / 1000)
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return date.formatted(.dateTime.hour().minute()) }
        if calendar.isDateInYesterday(date) { return String(localized: .chatTimeYesterday) }
        let days = calendar.dateComponents(
            [.day],
            from: calendar.startOfDay(for: date),
            to: calendar.startOfDay(for: Date())
        ).day ?? .max
        if days < 7 { return date.formatted(.dateTime.weekday(.abbreviated)) }
        if calendar.isDate(date, equalTo: Date(), toGranularity: .year) {
            return date.formatted(.dateTime.month(.defaultDigits).day())
        }
        return date.formatted(.dateTime.year().month(.defaultDigits).day())
    }
}

private struct InviteRow: View {
    let row: RoomRow
    let model: ChatModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(row.isDirect ? .chatInviteDirect(row.name) : .chatInviteRoom(row.name))
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.chatInviteAccept) { model.send(.joinRoom(roomId: row.roomId)) }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                Button(.chatInviteDecline) { model.send(.leaveRoom(roomId: row.roomId)) }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(WontonTheme.spacing.s16)
    }
}

/// 选一个人或输入用户名发起私聊，房间出现后打开。
private struct StartChatView: View {
    let model: ChatModel
    @State private var peer = ""

    var body: some View {
        let input = peer.trimmingCharacters(in: .whitespacesAndNewlines)
        let contacts = model.core.chatContacts(query: input)
        ScrollView {
            VStack(spacing: WontonTheme.spacing.s12) {
                TextField(String(localized: .chatStartField), text: $peer)
                    .textFieldStyle(.plain)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                Button {
                    model.openDirectChat(with: input)
                } label: {
                    Text(.chatStartSubmit).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                .disabled(input.isEmpty)
                if !contacts.isEmpty {
                    WontonGroup {
                        ForEach(Array(contacts.enumerated()), id: \.element.userId) { index, contact in
                            if index > 0 { WontonSeparator() }
                            Button {
                                model.openDirectChat(with: contact.userId)
                            } label: {
                                WontonRow(title: contact.name, showsForward: true)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .chatStartTitle))
    }
}

/// 房间列表预览的一行：正文原样，图片、文件等占位在这里写。
private extension MessageSummary {
    var line: String {
        switch self {
        case let .text(text): text
        case .files: String(localized: .messageSummaryFiles)
        case .image: String(localized: .messageSummaryImage)
        case .undecryptable: String(localized: .messageSummaryUndecryptable)
        case .redacted: String(localized: .messageSummaryRedacted)
        case let .other(kind): String(localized: .chatSummaryOther(kind))
        }
    }
}
