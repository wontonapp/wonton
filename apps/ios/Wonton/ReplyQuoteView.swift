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
    /// 竖条和名字的颜色（ADR-0109 第 9 节）：页面上用字色，画在自己气泡里时用气泡上的那一档。
    let emphasis: Color
    /// 输入框上方的那一份带叉、撑满一行；消息里的不带，按内容收缩，垫一层淡淡的底色。
    var cancel: (() -> Void)?
    /// 点了这条引用、核心还在往前翻页找原消息（ADR-0109 第 4 节）。
    var isLoading = false
    /// 点了引用跳回原消息（ADR-0109 第 4 节）；只有输入框上方那一份给。
    var jump: (() -> Void)?

    var body: some View {
        if cancel == nil {
            // 竖条画成盖在左边的一层，跟着内容的高度走。它不能是一个自己会伸缩的视图：气泡多出高度时会全被它吃掉，
            // 引用块就撑得老高（2026-11-20 真机实测）。
            row
                .padding(.leading, WontonTheme.size.quoteBar + 1 + WontonTheme.spacing.s8)
                .padding(.trailing, WontonTheme.spacing.s8)
                .padding(.vertical, WontonTheme.spacing.s4 / 2)
                .background(emphasis.opacity(WontonTheme.ratio.codeFill))
                .overlay(alignment: .leading) {
                    Rectangle().fill(emphasis).frame(width: WontonTheme.size.quoteBar + 1).accessibilityHidden(true)
                }
                .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.bubbleJoined - 2))
        } else {
            row
        }
    }

    private var row: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            if cancel != nil {
                Image(systemName: "arrowshape.turn.up.left")
                    .foregroundStyle(emphasis)
                    .accessibilityHidden(true)
            }
            if cancel != nil {
                Capsule()
                    .fill(emphasis)
                    .frame(width: WontonTheme.size.quoteBar + 1, height: WontonTheme.size.chip)
                    .accessibilityHidden(true)
            }
            if let jump {
                // 点引用本身跳回原消息（ADR-0109 第 4 节），叉还在右边、各管各的。
                Button(action: jump, label: { quoteContent })
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("replyQuoteJump")
            } else {
                quoteContent
            }
            if isLoading {
                ProgressView().controlSize(.small)
                    .accessibilityIdentifier("messageQuoteLoading")
            }
            if let cancel {
                Spacer(minLength: 0)
                Button(action: cancel) {
                    WontonIcon.close.view(size: WontonTheme.size.iconTiny)
                        .foregroundStyle(ink.opacity(WontonTheme.ratio.secondaryContent))
                        .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(.Message2.messageReplyCancel))
                .accessibilityIdentifier("replyCancel")
            }
        }
    }

    @ViewBuilder private var quoteContent: some View {
        switch quote {
        case let .ready(message):
            content(message)
        case .loading:
            ProgressView().controlSize(.small)
        case .unavailable:
            line(Text(.Message2.messageQuoteUnavailable))
        }
    }

    private func content(_ message: QuotedMessage) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(message.senderName)
                .wontonText(WontonTheme.typography.senderName)
                .foregroundStyle(emphasis)
                .lineLimit(1)
            switch message.content {
            case let .text(text):
                // 摘要按原文换行、最多两行（ADR-0109 第 8 节）：并成一行时表情和正文挤在一起，
                // 截断后和 Telegram、安卓里引用的样子都不一样；消息里和输入框上方行数也一样。
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
        .frame(maxWidth: cancel == nil ? nil : .infinity, alignment: .leading)
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
