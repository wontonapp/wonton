import SwiftUI
import WontonClient

/// 转发选会话（ADR-0073、ADR-0109 第 9 节）：顶上搜索，下面是带头像的会话，能选几个；选了以后底下出现发送。
/// 用聊天列表已经有的房间，只列已加入、能写的；按名字找在壳里做，只是在这张已经给了的名单里筛。
struct ForwardRoomSheet: View {
    let model: ChatModel
    let plugins: PluginsModel
    let rooms: [RoomRow]
    let send: ([RoomRow]) -> Void
    @State private var query = ""
    @State private var selected: [String] = []
    @Environment(\.wontonColors) private var colors

    private var listed: [RoomRow] {
        let wanted = query.trimmingCharacters(in: .whitespaces)
        return wanted.isEmpty ? rooms : rooms.filter { $0.name.localizedCaseInsensitiveContains(wanted) }
    }

    var body: some View {
        if rooms.isEmpty {
            Text(.Message0.messageForwardEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity)
                .padding(WontonTheme.spacing.pageInset)
        } else {
            VStack(spacing: WontonTheme.spacing.s8) {
                searchField
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(listed, id: \.roomId) { row($0) }
                    }
                }
                .frame(height: min(CGFloat(max(listed.count, 1)), Self.visibleRows) * Self.rowHeight)
                if !selected.isEmpty {
                    Button {
                        send(selected.compactMap { id in rooms.first { $0.roomId == id } })
                    } label: {
                        Text(verbatim: "\(String(localized: .Composer.composerSend)) (\(selected.count))")
                    }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .accessibilityIdentifier("forwardSend")
                }
            }
            .padding(.bottom, WontonTheme.spacing.s8)
        }
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
                        WontonIcon.check.view(size: WontonTheme.size.iconTiny).foregroundStyle(Color(colors.canvas))
                    }
                }
                .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .frame(height: Self.rowHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("forward-\(room.roomId)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private static let rowHeight = WontonTheme.size.topBar
    /// 列表最多露出这么多行，再多就在里面滚。
    private static let visibleRows: CGFloat = 6.5
}
