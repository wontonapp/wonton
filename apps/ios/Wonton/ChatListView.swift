import SwiftUI
import WontonClient

struct ChatListView: View {
    @Bindable var model: ChatModel
    @State private var isStartingChat = false
    @State private var peer = ""
    @State private var isShowingEncryption = false

    var body: some View {
        NavigationStack(path: $model.path) {
            List {
                if let status = statusText {
                    Section { Text(status).font(.footnote).foregroundStyle(.secondary) }
                }
                if let reminder = encryptionReminder {
                    Section { Button(reminder) { isShowingEncryption = true } }
                }
                ForEach(model.viewModel.rooms, id: \.roomId) { row in
                    if row.membership == .invited {
                        InviteRow(row: row, model: model)
                    } else {
                        NavigationLink(value: row.roomId) { RoomRowView(row: row) }
                    }
                }
            }
            .overlay {
                if model.viewModel.rooms.isEmpty, case .signedIn = model.viewModel.account {
                    ContentUnavailableView("还没有聊天", systemImage: "bubble.left.and.bubble.right")
                }
            }
            .navigationTitle("聊天")
            .navigationDestination(for: String.self) { roomId in
                // 提示条把路径直接换成另一个房间时，SwiftUI 会复用这个视图，@State 里还是旧房间的时间线。
                ChatRoomView(model: model, roomId: roomId, title: title(of: roomId))
                    .id(roomId)
            }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("加密", systemImage: "lock.shield") { isShowingEncryption = true }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("发起私聊", systemImage: "square.and.pencil") {
                        peer = ""
                        isStartingChat = true
                    }
                }
            }
            .alert("发起私聊", isPresented: $isStartingChat) {
                TextField("@hermes:wonton.localhost", text: $peer)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("发起") { model.send(.startDirectChat(userId: peer)) }
                Button("取消", role: .cancel) {}
            }
            .sheet(isPresented: $isShowingEncryption) {
                EncryptionView(model: model)
            }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
            .alert("操作失败", isPresented: isShowingProblem, presenting: model.viewModel.problem) { _ in
                Button("好") { model.send(.dismissChatProblem) }
            } message: { problem in
                Text(problem.message)
            }
        }
    }

    private var statusText: String? {
        switch model.viewModel.account {
        case .signedOut:
            return "还没有连上家里的服务器"
        case .signingIn:
            return "正在登录…"
        case let .failed(_, message):
            return message
        case .signedIn:
            switch model.viewModel.sync {
            case .idle, .starting:
                return "正在同步…"
            case .running:
                return nil
            case let .failed(_, message):
                return message
            }
        }
    }

    private var encryptionReminder: String? {
        let encryption = model.viewModel.encryption
        if encryption.recoveryKey != nil { return "恢复密钥还没确认保存" }
        if encryption.recovery == .incomplete { return "输入恢复密钥，才能解密以前的加密消息" }
        return nil
    }

    private func title(of roomId: String) -> String {
        model.viewModel.rooms.first { $0.roomId == roomId }?.name ?? roomId
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

private struct RoomRowView: View {
    let row: RoomRow

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 4) {
                    Text(row.name).font(.headline).lineLimit(1)
                    if row.isEncrypted {
                        Image(systemName: "lock.fill").font(.caption2).foregroundStyle(.secondary)
                    }
                }
                if let preview = row.preview {
                    Text(preview.isOwn ? "我：\(preview.text)" : preview.text)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if row.unreadCount > 0 {
                Text("\(row.unreadCount)")
                    .font(.caption.bold())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(.red))
            }
        }
    }
}

private struct InviteRow: View {
    let row: RoomRow
    let model: ChatModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("\(row.name) 邀请你\(row.isDirect ? "私聊" : "加入房间")").font(.headline)
            HStack {
                Button("接受") { model.send(.joinRoom(roomId: row.roomId)) }
                    .buttonStyle(.borderedProminent)
                Button("拒绝", role: .destructive) { model.send(.leaveRoom(roomId: row.roomId)) }
                    .buttonStyle(.bordered)
            }
        }
        .padding(.vertical, 4)
    }
}
