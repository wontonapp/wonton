import ImageIO
import SwiftUI
import WontonClient

/// 系统分享进来的分享页（ADR-0097）挂在主界面根上：没配对时也在。发出去后打开这个聊天看进度；关掉等于不发了。
struct ShareInPresenter: ViewModifier {
    let model: ShareInModel
    let chat: ChatModel
    let plugins: PluginsModel
    let openRoom: (String) -> Void

    private var presented: Binding<Bool> {
        Binding(get: { model.showsSheet }, set: { if !$0 { model.dismissed() } })
    }

    func body(content: Content) -> some View {
        content
            .wontonSheet(isPresented: presented) { close in
                if let share = model.viewModel.pending {
                    ShareInSheetContent(
                        share: share, sending: model.sending,
                        avatar: { RoomAvatar(row: $0, model: chat, plugins: plugins) },
                        pair: { model.pairLater() },
                        remove: { model.discard($0) },
                        send: { roomId, body in
                            Task {
                                if await model.send(to: roomId, body: body) { close { openRoom(roomId) } }
                            }
                        }
                    )
                }
            }
            .wontonDialog(problem: model.problem, isPresented: Binding(
                get: { model.problem != nil }, set: { if !$0 { model.problem = nil } }
            ))
            .task { await model.observe() }
    }
}

/// 分享页面板里的内容；截图测试直接画它。
struct ShareInSheetContent<Avatar: View>: View {
    let share: PendingShare
    let sending: Bool
    @ViewBuilder let avatar: (RoomRow) -> Avatar
    let pair: () -> Void
    let remove: (String) -> Void
    let send: (String, String) -> Void
    @State private var text: String
    @State private var picked: String?
    @State private var choosing = false

    init(share: PendingShare, sending: Bool, @ViewBuilder avatar: @escaping (RoomRow) -> Avatar,
         pair: @escaping () -> Void, remove: @escaping (String) -> Void, send: @escaping (String, String) -> Void) {
        self.share = share
        self.sending = sending
        self.avatar = avatar
        self.pair = pair
        self.remove = remove
        self.send = send
        _text = State(initialValue: share.text)
    }

    private var rooms: [RoomRow] {
        if case let .ready(rooms, _) = share.destinations { return rooms }
        return []
    }

    private var chosen: RoomRow? {
        guard case let .ready(rooms, suggested) = share.destinations else { return nil }
        return rooms.first { $0.roomId == (picked ?? suggested) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            ShareInDestinationView(destinations: share.destinations, chosen: chosen, avatar: avatar,
                                   choose: { choosing.toggle() }, pair: pair)
            if choosing, !rooms.isEmpty {
                VStack(spacing: 0) {
                    ForEach(rooms, id: \.roomId) { room in
                        ShareInRoomRow(row: room, avatar: avatar) {
                            picked = room.roomId
                            choosing = false
                        }
                    }
                }
            }
            if !share.items.isEmpty { ShareInItemStrip(items: share.items, remove: remove) }
            if share.dropped > 0 {
                WontonNotice(icon: .info, title: String(localized: .shareInDropped(Int(share.dropped))), isError: false)
            }
            TextField(String(localized: .Composer.composerMessage), text: $text, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.plain)
                .wontonField()
            Button {
                if let chosen { send(chosen.roomId, text) }
            } label: {
                Text(.Composer.composerSend).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(chosen == nil || sending || (text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && share.items.isEmpty))
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.bottom, WontonTheme.spacing.s16)
    }
}

private struct ShareInDestinationView<Avatar: View>: View {
    let destinations: ShareInDestinations
    let chosen: RoomRow?
    let avatar: (RoomRow) -> Avatar
    let choose: () -> Void
    let pair: () -> Void

    var body: some View {
        switch destinations {
        case .notPaired:
            WontonNotice(icon: .connection, title: String(localized: .shareInNotPaired), isError: false) {
                Button(String(localized: .shareInPair), action: pair)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        case .loading:
            ProgressView().frame(maxWidth: .infinity)
        case let .unavailable(message):
            ProblemNotice(problem: Problem(title: String(localized: .shareInChatUnavailable), detail: message))
        case let .ready(rooms, _):
            if rooms.isEmpty {
                WontonNotice(icon: .chat, title: String(localized: .shareInNoChats), isError: false)
            } else {
                ShareInRoomRow(row: chosen, avatar: avatar, showsExpand: true, action: choose)
            }
        }
    }
}

/// 去向行：选中的聊天的头像和名字；还没选时提示去选。
private struct ShareInRoomRow<Avatar: View>: View {
    let row: RoomRow?
    let avatar: (RoomRow) -> Avatar
    var showsExpand = false
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s12) {
                if let row { avatar(row) }
                Text(row?.name ?? String(localized: .shareInChooseChat))
                    .wontonText(WontonTheme.typography.itemTitle)
                    .lineLimit(1)
                    .foregroundStyle(Color(row == nil ? colors.textSecondary : colors.textPrimary))
                Spacer(minLength: 0)
                if showsExpand {
                    WontonIcon.expand.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                        .accessibilityHidden(true)
                }
            }
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// 分享进来的条目：照片是缩略图，文件是图标和名字，每项右上角能删。
private struct ShareInItemStrip: View {
    let items: [ShareInItem]
    let remove: (String) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                ForEach(items, id: \.id) { item in
                    ShareInItemTile(item: item) { remove(item.id) }
                }
            }
        }
    }
}

private struct ShareInItemTile: View {
    let item: ShareInItem
    let remove: () -> Void
    @State private var thumbnail: UIImage?
    @Environment(\.wontonColors) private var colors
    @Environment(\.displayScale) private var displayScale

    private var isImage: Bool {
        if case .image = item.kind { return true }
        return false
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            ZStack(alignment: .topTrailing) {
                tile
                Button(action: remove) {
                    WontonIcon.close.view(size: WontonTheme.size.iconTiny)
                        .foregroundStyle(Color(colors.textPrimary))
                        .padding(2)
                        .background(Circle().fill(Color(colors.surface)))
                        .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: .shareInRemoveItem(item.name)))
            }
            if !isImage {
                Text(item.name)
                    .wontonText(WontonTheme.typography.caption)
                    .lineLimit(1)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .frame(width: WontonTheme.size.previewIcon)
        .task(id: item.path) { thumbnail = isImage ? await Self.load(item.path, scale: displayScale) : nil }
    }

    private var tile: some View {
        ZStack {
            RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle))
            if let thumbnail {
                Image(uiImage: thumbnail)
                    .resizable()
                    .scaledToFill()
                    .accessibilityLabel(item.name)
            } else {
                (isImage ? WontonIcon.image : WontonIcon.file).view()
                    .foregroundStyle(Color(colors.textSecondary))
                    .accessibilityHidden(true)
            }
        }
        .frame(width: WontonTheme.size.previewIcon, height: WontonTheme.size.previewIcon)
        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
    }

    /// 按格子大小缩着解码，不把原图整张读进内存；解不出来时显示图标。
    private static func load(_ path: String, scale: CGFloat) async -> UIImage? {
        let side = WontonTheme.size.previewIcon * scale
        return await Task.detached(priority: .userInitiated) {
            guard let source = CGImageSourceCreateWithURL(URL(filePath: path) as CFURL, nil) else { return nil }
            let options: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: side
            ]
            return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary).map { UIImage(cgImage: $0) }
        }.value
    }
}
