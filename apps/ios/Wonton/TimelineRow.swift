import SwiftUI
import WontonClient

struct TimelineRow: View {
    let item: TimelineItem
    let core: ClientCore
    /// 私聊里对方是谁看标题就知道，不在每条消息上重复名字。
    let showsSender: Bool
    /// 重发、删除发送失败的这一条。
    let retry: () -> Void
    let cancel: () -> Void
    /// 回答这张确认卡片。
    let answer: (String, Answer) -> Void

    var body: some View {
        switch item.content {
        case let .message(message):
            MessageBubble(message: message, core: core, showsSender: showsSender, retry: retry, cancel: cancel,
                          answer: answer)
        case let .notice(text):
            Text(text).font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity)
        case let .dateDivider(timestampMs):
            Text(Self.date(timestampMs), format: .dateTime.year().month().day().weekday())
                .font(.caption.bold())
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
        case .readMarker:
            HStack {
                VStack { Divider() }
                Text("以下是新消息").font(.caption2).foregroundStyle(.red)
                VStack { Divider() }
            }
        case .timelineStart:
            Text("以上是聊天的开头").font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity)
        }
    }

    static func date(_ timestampMs: UInt64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(timestampMs) / 1000)
    }
}

private struct MessageBubble: View {
    let message: TimelineMessage
    let core: ClientCore
    let showsSender: Bool
    let retry: () -> Void
    let cancel: () -> Void
    let answer: (String, Answer) -> Void

    var body: some View {
        HStack {
            if message.isOwn { Spacer(minLength: 48) }
            VStack(alignment: message.isOwn ? .trailing : .leading, spacing: 2) {
                if showsSender, !message.isOwn {
                    Text(message.senderName).font(.caption).foregroundStyle(.secondary)
                }
                content
                    .padding(10)
                    .background(
                        RoundedRectangle(cornerRadius: 14)
                            .fill(message.isOwn ? Color.accentColor.opacity(0.2) : Color.secondary.opacity(0.12))
                    )
                footer
            }
            if !message.isOwn { Spacer(minLength: 48) }
        }
    }

    @ViewBuilder private var content: some View {
        switch message.body {
        case let .text(body, files):
            TextMessageBody(text: body, files: files)
        case let .image(image):
            MediaImage(image: image, core: core, sendState: message.sendState)
        case let .confirmation(view):
            ConfirmationCardView(view: view) { answer(view.card.id, $0) }
        case .undecryptable:
            Label("无法解密：本设备还没有这条消息的密钥", systemImage: "lock.slash").font(.footnote)
        case .redacted:
            Text("消息已撤回").italic().foregroundStyle(.secondary)
        case let .unsupported(kind):
            Text("暂不支持显示的消息（\(kind)）").font(.footnote).foregroundStyle(.secondary)
        }
    }

    private var footer: some View {
        HStack(spacing: 4) {
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
                    Label(recoverable ? "发送失败，会自动重试" : "发送失败：\(reason)",
                          systemImage: "exclamationmark.circle")
                }
                .tint(.red)
                .foregroundStyle(.red)
            case .sent:
                if message.isOwn, !message.readBy.isEmpty { Text("已读") }
            }
        }
        .font(.caption2)
        .foregroundStyle(.secondary)
    }
}

private struct MediaImage: View {
    let image: ImageContent
    let core: ClientCore
    let sendState: SendState
    @State private var preview: Result<UIImage, MediaFailure>?
    @State private var isShowingOriginal = false

    var body: some View {
        Group {
            switch preview {
            case let .success(loaded):
                Image(uiImage: loaded).resizable().scaledToFit()
            case let .failure(failure):
                Label(failure.message, systemImage: "photo.badge.exclamationmark").font(.footnote)
            case nil:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: 240)
        .aspectRatio(ratio, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 8))
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
        .onTapGesture { isShowingOriginal = true }
        .accessibilityAddTraits(.isButton)
        .task(id: image.thumbnail ?? image.media) {
            preview = await MediaFailure.load(image.thumbnail ?? image.media, from: core)
        }
        .fullScreenCover(isPresented: $isShowingOriginal) {
            OriginalImage(image: image, core: core)
        }
    }

    private var ratio: CGFloat? {
        guard let width = image.width, let height = image.height, width > 0, height > 0 else { return nil }
        return CGFloat(width) / CGFloat(height)
    }
}

private struct OriginalImage: View {
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
                    ContentUnavailableView("取不到原图", systemImage: "photo.badge.exclamationmark",
                                           description: Text(failure.message))
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
