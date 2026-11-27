import SwiftUI
import WontonClient

extension View {
    /// 房间页上的转发面板：`item` 不为空时打开，转发的就是这一条。
    func forwardSheet(item: Binding<String?>, model: ChatModel, plugins: PluginsModel, from source: TimelineTarget)
        -> some View {
        modifier(ForwardPresentation(item: item, model: model, plugins: plugins, source: source))
    }
}

/// 点了发送先收面板，收好了再发：只发一个会话时要跳过去，跳转不能和面板收起撞上。
private struct ForwardPresentation: ViewModifier {
    @Binding var item: String?
    let model: ChatModel
    let plugins: PluginsModel
    let source: TimelineTarget
    @State private var pending: PendingForward?

    /// 只列已加入、能写的房间。
    private var rooms: [RoomRow] {
        model.viewModel.rooms.filter { $0.membership == .joined && !$0.isReadOnly }
    }

    func body(content: Content) -> some View {
        content.background {
            Color.clear.sheet(isPresented: Binding(get: { item != nil }, set: { if !$0 { item = nil } }),
                              onDismiss: sendPending) {
                ForwardRoomSheet(model: model, plugins: plugins, rooms: rooms) { targets, comment in
                    let chosen = "chosen rooms=\(targets.count) comment=\(comment.count)"
                    walkLog.notice("forward \(chosen, privacy: .public)")
                    if let item { pending = PendingForward(item: item, rooms: targets, comment: comment) }
                    item = nil
                }
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
            }
        }
    }

    private func sendPending() {
        walkLog.notice("forward dismiss pending=\(self.pending != nil, privacy: .public)")
        guard let pending else { return }
        self.pending = nil
        model.forward(pending.item, from: source, to: pending.rooms, comment: pending.comment)
    }
}

private struct PendingForward {
    let item: String
    let rooms: [RoomRow]
    let comment: String
}

/// 转发选会话（ADR-0073、ADR-0109 第 9 节）：顶上搜索，下面是带头像的会话，能选几个；选了以后底下出现附言框和发送键。
/// 用聊天列表已经有的房间；按名字找在壳里做，只是在这张已经给了的名单里筛。
struct ForwardRoomSheet: View {
    let model: ChatModel
    let plugins: PluginsModel
    let rooms: [RoomRow]
    let send: (_ rooms: [RoomRow], _ comment: String) -> Void
    @State private var query = ""
    @State private var selected: [String] = []
    @State private var comment = ""
    @Environment(\.wontonColors) private var colors

    private var listed: [RoomRow] {
        let wanted = query.trimmingCharacters(in: .whitespaces)
        return wanted.isEmpty ? rooms : rooms.filter { $0.name.localizedCaseInsensitiveContains(wanted) }
    }

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            Text(.Message0.messageForwardTitle)
                .wontonText(WontonTheme.typography.barTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .padding(.top, WontonTheme.spacing.s24)
                .accessibilityAddTraits(.isHeader)
            if rooms.isEmpty {
                Text(.Message0.messageForwardEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                searchField
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(listed, id: \.roomId) { row($0) }
                    }
                }
                .scrollDismissesKeyboard(.interactively)
            }
        }
        // 底下那一条贴着面板底，列表从它下面滚过去；键盘弹起时跟着上来。
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !selected.isEmpty {
                CaptionSendBar(
                    placeholder: String(localized: .Message0.messageForwardComment), text: $comment,
                    count: selected.count,
                    send: { send(selected.compactMap { id in rooms.first { $0.roomId == id } }, comment) },
                    id: "forward"
                )
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeOut(duration: WontonTheme.motion.state), value: selected.isEmpty)
    }

    private var searchField: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.search.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            TextField(String(localized: .Store.storeSearchPrompt), text: $query)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .accessibilityIdentifier("forwardSearch")
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.chatControl)
        .background(Color(colors.surfaceSubtle), in: Capsule())
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }

    private func row(_ room: RoomRow) -> some View {
        let isSelected = selected.contains(room.roomId)
        return Button {
            if let index = selected.firstIndex(of: room.roomId) {
                selected.remove(at: index)
            } else {
                selected.append(room.roomId)
            }
        } label: {
            HStack(spacing: WontonTheme.spacing.s12) {
                RoomAvatar(row: room, model: model, plugins: plugins, size: WontonTheme.size.chatControl)
                    .clipShape(Circle())
                Text(room.name)
                    .wontonText(WontonTheme.typography.body)
                    .lineLimit(1)
                    .foregroundStyle(Color(colors.textPrimary))
                Spacer(minLength: 0)
                ZStack {
                    Circle().strokeBorder(Color(isSelected ? colors.accentInk : colors.separator), lineWidth: 1.5)
                    if isSelected {
                        Circle().fill(Color(colors.accentInk))
                        WontonIcon.check.view(size: WontonTheme.size.iconTiny).wontonIconMono()
                            .foregroundStyle(Color(colors.canvas))
                    }
                }
                .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .frame(height: WontonTheme.size.topBar)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("forward-\(room.roomId)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
