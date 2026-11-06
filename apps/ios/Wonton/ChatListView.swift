import SwiftUI
import WontonClient

struct ChatListView: View {
    @Bindable var model: ChatModel
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors

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
                    ForEach(Array(model.viewModel.rooms.enumerated()), id: \.element.roomId) { index, row in
                        if index > 0 {
                            WontonSeparator().padding(.leading, WontonTheme.size.avatar + WontonTheme.spacing.s12)
                        }
                        if row.membership == .invited {
                            InviteRow(row: row, model: model)
                        } else {
                            NavigationLink(value: ChatRoute.room(row.roomId)) {
                                RoomRowView(row: row, plugins: plugins)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            .overlay {
                if isEmpty { NoChats() }
            }
            .safeAreaInset(edge: .bottom, alignment: .trailing, spacing: 0) {
                newChatButton
                    .padding(.trailing, WontonTheme.spacing.s16)
                    .padding(.bottom, WontonTheme.spacing.s12)
            }
            .wontonRootScreen("聊天", hasActions: false) { EmptyView() }
            .navigationDestination(for: ChatRoute.self) { route in
                switch route {
                case let .room(roomId):
                    // 提示条把路径直接换成另一个房间时，SwiftUI 会复用这个视图，@State 里还是旧房间的时间线。
                    ChatRoomView(model: model, roomId: roomId, title: model.title(of: roomId))
                        .id(roomId)
                case .chatBackup:
                    ChatBackupView(model: model)
                case .startChat:
                    StartChatView(model: model)
                }
            }
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
        .accessibilityLabel("新会话")
    }

    @ViewBuilder private var notices: some View {
        let status = ChatStatus(model.viewModel)
        let reminder = model.viewModel.encryption.backupReminder?.text
        if status != nil || reminder != nil {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                status.map(ChatStatusView.init)
                if let reminder {
                    WontonGroup {
                        NavigationLink(value: ChatRoute.chatBackup) { WontonRow(title: reminder, showsForward: true) }
                            .buttonStyle(.plain)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    /// 同步好之前列表是空的不代表没有聊天。
    private var isEmpty: Bool {
        guard model.viewModel.rooms.isEmpty, case .signedIn = model.viewModel.account,
              case .running = model.viewModel.sync else { return false }
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
            self = .progress("正在登录…")
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
            WontonNotice(icon: .connection, title: "还没有连上家里的服务器", isError: false)
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
    let tab: AppTab
    @Binding var path: NavigationPath
    @State private var depth = 0

    func body(content: Content) -> some View {
        content
            .navigationDestination(for: VisitedRoom.self) { room in
                ChatRoomView(model: model, roomId: room.roomId, title: model.title(of: room.roomId))
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
        Text("还没有聊天")
            .wontonText(WontonTheme.typography.itemTitle)
            .foregroundStyle(Color(colors.textPrimary))
    }
}

private struct RoomRowView: View {
    let row: RoomRow
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            RoomAvatar(row: row, plugins: plugins)
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
                                .accessibilityLabel("已加密")
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
                        Text(preview.isOwn ? "我：\(preview.body.line)" : preview.body.line)
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
                            .accessibilityLabel("\(row.unreadCount) 条未读")
                    }
                }
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .contentShape(Rectangle())
    }
}

/// Bot 用它的应用的图标，其他人用名字的第一个字。
private struct RoomAvatar: View {
    let row: RoomRow
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors

    private var plugin: InstalledPlugin? {
        guard let bot = row.directUserId else { return nil }
        return plugins.viewModel.installed?.plugins.first { $0.botUserId == bot }
    }

    var body: some View {
        let size = WontonTheme.size.avatar
        if let plugin {
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
        if calendar.isDateInYesterday(date) { return "昨天" }
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
            Text("\(row.name) 邀请你\(row.isDirect ? "私聊" : "加入房间")")
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button("接受") { model.send(.joinRoom(roomId: row.roomId)) }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                Button("拒绝") { model.send(.leaveRoom(roomId: row.roomId)) }
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
                TextField("用户名或 Matrix ID", text: $peer)
                    .textFieldStyle(.plain)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                Button {
                    model.openDirectChat(with: input)
                } label: {
                    Text("发起").frame(maxWidth: .infinity)
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
        .wontonPushedScreen("发起私聊")
    }
}

/// 房间列表预览的一行：正文原样，图片、文件等占位在这里写。
private extension MessageSummary {
    var line: String {
        switch self {
        case let .text(text): text
        case .files: "[文件]"
        case .image: "[图片]"
        case .undecryptable: "[加密消息]"
        case .redacted: "[已撤回]"
        case let .other(kind): "[\(kind)]"
        }
    }
}
