import SwiftUI
import WontonClient

/// 转发选房间（ADR-0073）：用聊天列表已经有的房间，只列已加入、能写的。
struct ForwardRoomSheet: View {
    let model: ChatModel
    let plugins: PluginsModel
    let rooms: [RoomRow]
    let choose: (RoomRow) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if rooms.isEmpty {
            Text(.Message0.messageForwardEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity)
                .padding(WontonTheme.spacing.pageInset)
        } else {
            VStack(spacing: 0) {
                ForEach(rooms, id: \.roomId) { room in
                    Button {
                        choose(room)
                    } label: {
                        HStack(spacing: WontonTheme.spacing.s12) {
                            RoomAvatar(row: room, model: model, plugins: plugins, size: WontonTheme.size.smallButton)
                            Text(room.name)
                                .wontonText(WontonTheme.typography.body)
                                .lineLimit(1)
                                .foregroundStyle(Color(colors.textPrimary))
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .frame(minHeight: WontonTheme.size.target)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("forward-\(room.roomId)")
                }
            }
        }
    }
}
