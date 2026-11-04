import SwiftUI
import WontonClient

struct ChatListView: View {
    @Bindable var model: ChatModel

    var body: some View {
        NavigationStack(path: $model.path) {
            ScrollView {
                LazyVStack(spacing: 0) {
                    notices
                    ForEach(Array(model.viewModel.rooms.enumerated()), id: \.element.roomId) { index, row in
                        if index > 0 { WontonSeparator() }
                        if row.membership == .invited {
                            InviteRow(row: row, model: model)
                        } else {
                            NavigationLink(value: ChatRoute.room(row.roomId)) { RoomRowView(row: row) }
                                .buttonStyle(.plain)
                        }
                    }
                }
            }
            .overlay {
                if isEmpty { NoChats() }
            }
            .wontonRootScreen("聊天") {
                WontonIconLink(icon: .compose, label: "发起私聊", value: ChatRoute.startChat)
            }
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
            .modifier(ChatDialogs(model: model))
        }
    }

    @ViewBuilder private var notices: some View {
        let status = ChatStatus(model.viewModel)
        let reminder = model.viewModel.encryption.backupReminder
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

/// 聊天操作的结果。备份页也能从设置进，那边的导航栈也要挂一份。
struct ChatDialogs: ViewModifier {
    let model: ChatModel

    func body(content: Content) -> some View {
        content
            .wontonDialog("操作被拒绝", message: model.rejection, isPresented: isShowingRejection)
            .wontonDialog(
                "操作失败",
                message: model.viewModel.problem?.message,
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
    case failed(String)

    init?(_ viewModel: ChatViewModel) {
        switch viewModel.account {
        case .signedOut:
            self = .signedOut
        case .signingIn:
            self = .progress("正在登录…")
        case let .failed(_, message):
            self = .failed(message)
        case .signedIn:
            switch viewModel.sync {
            case .idle, .starting:
                self = .progress("正在同步…")
            case .running:
                return nil
            case let .failed(_, message):
                self = .failed(message)
            }
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
        case let .failed(message):
            WontonNotice(icon: .warning, title: message)
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
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
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
                if let preview = row.preview {
                    Text(preview.isOwn ? "我：\(preview.text)" : preview.text)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
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
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .contentShape(Rectangle())
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

/// 输入对方的 Matrix ID 发起私聊，房间出现后打开。
private struct StartChatView: View {
    let model: ChatModel
    @State private var peer = ""

    var body: some View {
        let userId = peer.trimmingCharacters(in: .whitespacesAndNewlines)
        ScrollView {
            VStack(spacing: WontonTheme.spacing.s12) {
                TextField("@hermes:wonton.localhost", text: $peer)
                    .textFieldStyle(.plain)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                    .accessibilityLabel("对方的 Matrix ID")
                Button {
                    model.openDirectChat(with: userId)
                } label: {
                    Text("发起").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                .disabled(userId.isEmpty)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("发起私聊")
    }
}
