import SwiftUI
import WontonClient

extension ReplyQuote {
    /// 发送时交回核心的 `reply_to`。
    var eventId: String {
        switch self {
        case let .loading(eventId), let .unavailable(eventId): eventId
        case let .ready(message): message.eventId
        }
    }
}

/// 引用的原消息（ADR-0073）：输入框上方的回复状态、消息正文上方的引用都用它。类别由核心给，这里只画。
struct ReplyQuoteView: View {
    let quote: ReplyQuote
    let core: ClientCore
    /// 画在自己气泡里时用气泡的文字颜色。
    let ink: Color
    /// 输入框上方的那一份带叉，消息里的不带。
    var cancel: (() -> Void)?

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            Capsule()
                .fill(ink.opacity(WontonTheme.ratio.secondaryContent))
                .frame(width: WontonTheme.size.quoteBar, height: WontonTheme.size.chip)
                .accessibilityHidden(true)
            Group {
                switch quote {
                case let .ready(message):
                    content(message)
                case .loading:
                    ProgressView().controlSize(.small)
                case .unavailable:
                    line(Text(.Message2.messageQuoteUnavailable))
                }
            }
            Spacer(minLength: 0)
            if let cancel {
                Button(action: cancel) {
                    WontonIcon.close.view(size: WontonTheme.size.iconTiny)
                        .foregroundStyle(ink)
                        .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(.Message2.messageReplyCancel))
                .accessibilityIdentifier("replyCancel")
            }
        }
    }

    private func content(_ message: QuotedMessage) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(message.senderName)
                .wontonText(WontonTheme.typography.caption)
                .opacity(WontonTheme.ratio.secondaryContent)
            switch message.content {
            case let .text(text):
                Text(text)
                    .wontonText(WontonTheme.typography.supporting)
                    .lineLimit(2)
            case let .image(thumbnail):
                QuotedThumbnail(media: thumbnail, core: core)
                    .padding(.top, WontonTheme.spacing.s4)
            case .redacted:
                line(Text(.Message2.messageQuoteRedacted))
            case .undecryptable:
                line(Text(.Message2.messageQuoteUndecryptable))
            case .other:
                line(Text(.Message1.messageQuoteOther))
            }
        }
        .foregroundStyle(ink)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func line(_ text: Text) -> some View {
        text.wontonText(WontonTheme.typography.supporting).lineLimit(1)
    }
}

private struct QuotedThumbnail: View {
    let media: MediaHandle?
    let core: ClientCore
    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                WontonIcon.image.view(size: WontonTheme.size.iconTiny)
            }
        }
        .frame(width: WontonTheme.size.chip, height: WontonTheme.size.chip)
        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control / 2))
        .task(id: media?.token) {
            guard let media, case let .success(loaded) = await MediaFailure.load(media, from: core) else { return }
            image = loaded
        }
    }
}
