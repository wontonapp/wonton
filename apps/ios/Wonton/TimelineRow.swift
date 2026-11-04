import SwiftUI
import WontonClient

struct TimelineRow: View {
    let item: TimelineItem
    let core: ClientCore
    /// 私聊里对方是谁看标题就知道，不在每条消息上重复名字。
    let showsSender: Bool
    /// 和上一条、下一条是不是同一个人连着发的。
    let joinsPrevious: Bool
    let joinsNext: Bool
    /// 重发、删除发送失败的这一条。
    let retry: () -> Void
    let cancel: () -> Void
    /// 回答这张确认卡片。
    let answer: (String, Answer) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch item.content {
        case let .message(message):
            MessageBubble(
                message: message, core: core, showsSender: showsSender && !joinsPrevious,
                joinsPrevious: joinsPrevious, joinsNext: joinsNext, retry: retry, cancel: cancel, answer: answer
            )
        case let .notice(text):
            note(Text(text))
        case let .dateDivider(timestampMs):
            note(Text(Self.date(timestampMs), format: .dateTime.year().month().day().weekday()))
        case .readMarker:
            HStack(spacing: WontonTheme.spacing.s8) {
                edge
                Text("以下是新消息")
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.accentInk))
                edge
            }
        case .timelineStart:
            note(Text("以上是聊天的开头"))
        }
    }

    private var edge: some View {
        Rectangle().fill(Color(colors.separator)).frame(height: WontonTheme.size.hairline)
    }

    private func note(_ text: Text) -> some View {
        text
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
    }

    static func date(_ timestampMs: UInt64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(timestampMs) / 1000)
    }
}

private struct MessageBubble: View {
    let message: TimelineMessage
    let core: ClientCore
    let showsSender: Bool
    let joinsPrevious: Bool
    let joinsNext: Bool
    let retry: () -> Void
    let cancel: () -> Void
    let answer: (String, Answer) -> Void
    @Environment(\.wontonColors) private var colors

    /// 自己的气泡靠右，相接的是右边的角；别人的相反。
    private var shape: UnevenRoundedRectangle {
        let full = WontonTheme.radii.bubble
        let joined = WontonTheme.radii.bubbleJoined
        let top = joinsPrevious ? joined : full
        let bottom = joinsNext ? joined : full
        return UnevenRoundedRectangle(
            topLeadingRadius: message.isOwn ? full : top,
            bottomLeadingRadius: message.isOwn ? full : bottom,
            bottomTrailingRadius: message.isOwn ? bottom : full,
            topTrailingRadius: message.isOwn ? top : full,
            style: .continuous
        )
    }

    var body: some View {
        HStack(spacing: 0) {
            if message.isOwn { Spacer(minLength: WontonTheme.spacing.s48) }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                if showsSender, !message.isOwn {
                    Text(message.senderName)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                bubble
            }
            if !message.isOwn { Spacer(minLength: WontonTheme.spacing.s48) }
        }
    }

    @ViewBuilder private var bubble: some View {
        // Grid：气泡和最宽的一行一样宽，正文靠左、时间靠右。VStack 做不到，撑满又会把短消息的气泡也撑满。
        let body = Grid(alignment: .leading, verticalSpacing: WontonTheme.spacing.s4) {
            content
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(textColor)
            footer.gridCellAnchor(.trailing)
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .padding(.vertical, WontonTheme.spacing.s8)
        if message.isOwn {
            body.accentFill(shape)
        } else {
            body.background(shape.fill(Color(colors.surfaceSubtle)))
        }
    }

    private var textColor: Color {
        Color(message.isOwn ? colors.textOnAccentFill : colors.textPrimary)
    }

    @ViewBuilder private var content: some View {
        switch message.body {
        case let .text(_, blocks, files):
            TextMessageBody(blocks: blocks, ink: textColor, files: files)
        case let .image(image):
            MediaImage(image: image, core: core, sendState: message.sendState)
        case let .confirmation(view):
            ConfirmationCardView(view: view) { answer(view.card.id, $0) }
        case .undecryptable:
            Label {
                Text("无法解密：本设备还没有这条消息的密钥")
            } icon: {
                WontonIcon.lockKey.view(size: WontonTheme.size.iconSmall)
            }
            .wontonText(WontonTheme.typography.supporting)
        case .redacted:
            Text("消息已撤回").italic().opacity(WontonTheme.ratio.secondaryContent)
        case let .unsupported(kind):
            Text("暂不支持显示的消息（\(kind)）")
                .wontonText(WontonTheme.typography.supporting)
                .opacity(WontonTheme.ratio.secondaryContent)
        }
    }

    private var footer: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            if message.edited { Text("已编辑") }
            Text(TimelineRow.date(message.timestampMs), format: .dateTime.hour().minute())
            switch message.sendState {
            case .sending:
                Text("发送中")
            case let .failed(reason, recoverable):
                Menu {
                    Button("重发", action: retry)
                    Button("删除", role: .destructive, action: cancel)
                } label: {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        WontonIcon.warningCircle.view(size: WontonTheme.size.iconTiny)
                        Text(recoverable ? "发送失败，会自动重试" : "发送失败：\(reason)")
                    }
                    .foregroundStyle(Color(colors.danger))
                }
            case .sent:
                if message.isOwn, !message.readBy.isEmpty { Text("已读") }
            }
        }
        .wontonText(WontonTheme.typography.caption)
        .foregroundStyle(Color(message.isOwn ? colors.accentInkOnFill : colors.textSecondary))
    }
}

private struct MediaImage: View {
    let image: ImageContent
    let core: ClientCore
    let sendState: SendState
    @Environment(RoomPresenter.self) private var presenter
    @State private var preview: Result<UIImage, MediaFailure>?

    var body: some View {
        Group {
            switch preview {
            case let .success(loaded):
                Image(uiImage: loaded).resizable().scaledToFit()
            case let .failure(failure):
                Label {
                    Text(failure.message)
                } icon: {
                    WontonIcon.imageBroken.view(size: WontonTheme.size.iconSmall)
                }
                .wontonText(WontonTheme.typography.supporting)
            case nil:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: WontonTheme.size.messageImage)
        .aspectRatio(ratio, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        .overlay(alignment: .bottom) {
            if case let .sending(progress) = sendState {
                if let progress {
                    ProgressView(value: progress)
                        .progressViewStyle(.linear)
                        .frame(maxWidth: .infinity)
                        .padding(4)
                } else {
                    ProgressView()
                        .padding(4)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { presenter.showOriginal(image) }
        .accessibilityAddTraits(.isButton)
        .task(id: image.thumbnail ?? image.media) {
            preview = await MediaFailure.load(image.thumbnail ?? image.media, from: core)
        }
    }

    private var ratio: CGFloat? {
        guard let width = image.width, let height = image.height, width > 0, height > 0 else { return nil }
        return CGFloat(width) / CGFloat(height)
    }
}

struct OriginalImage: View {
    let image: ImageContent
    let core: ClientCore
    @Environment(\.dismiss) private var dismiss
    @State private var original: Result<UIImage, MediaFailure>?

    var body: some View {
        NavigationStack {
            Group {
                switch original {
                case let .success(loaded):
                    Image(uiImage: loaded).resizable().scaledToFit()
                case let .failure(failure):
                    WontonNotice(icon: .imageBroken, title: "取不到原图", lines: [failure.message])
                        .padding(WontonTheme.spacing.pageInset)
                case nil:
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.black)
            .navigationTitle(image.filename)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbarBackground(.black, for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .toolbar { Button("完成") { dismiss() } }
        }
        .task { original = await MediaFailure.load(image.media, from: core) }
    }
}

private struct MediaFailure: Error {
    let message: String

    static func load(_ media: MediaHandle, from core: ClientCore) async -> Result<UIImage, MediaFailure> {
        do {
            let data = try await core.loadMedia(media: media)
            guard let image = UIImage(data: data) else { return .failure(MediaFailure(message: "图片格式不支持")) }
            return .success(image)
        } catch {
            return .failure(MediaFailure(message: String(describing: error)))
        }
    }
}
