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
    let cardAction: (Action) -> Void
    /// 进入回复状态，引用由核心随菜单给。
    let reply: (QuotedMessage) -> Void
    let forward: () -> Void
    let redact: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch item.content {
        case let .message(message):
            MessageBubble(
                message: message, core: core, showsSender: showsSender && !joinsPrevious,
                joinsPrevious: joinsPrevious, joinsNext: joinsNext, retry: retry, cancel: cancel,
                cardAction: cardAction, reply: reply, forward: forward, redact: redact
            )
        case let .notice(text):
            note(Text(text))
        case let .agentRun(_, _, _, _, _, _, _, body, _, _):
            // 一轮 Agent 任务的进度行（ADR-0082）：完整的样子在 App 壳那个 PR 里做，先按普通的
            // Bot 提示消息显示 `body`——和其他不认识这个格式的客户端一样。
            note(Text(body))
        case let .profileChange(_, name, previousName, _):
            if let previousName {
                note(Text(.timelineRenamed(previousName, name)))
            } else {
                note(Text(.timelineAvatarChanged(name)))
            }
        case let .dateDivider(timestampMs):
            note(Text(Self.date(timestampMs), format: .dateTime.year().month().day().weekday()))
        case .readMarker:
            HStack(spacing: WontonTheme.spacing.s8) {
                edge
                Text(.timelineReadMarker)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.accentInk))
                edge
            }
        case .timelineStart:
            note(Text(.timelineStart))
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

    static func time(_ timestampMs: UInt64) -> String {
        date(timestampMs).formatted(.dateTime.hour().minute())
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
    let cardAction: (Action) -> Void
    let reply: (QuotedMessage) -> Void
    let forward: () -> Void
    let redact: () -> Void
    @State private var isChoosingFailed = false
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
                    // 系统的菜单自带预览、背景模糊和材质（ADR-0029）。
                    .contentShape(.contextMenuPreview, shape)
                    .contextMenu {
                        ForEach(Array(message.actions.enumerated()), id: \.offset) { _, action in
                            menuItem(action)
                        }
                    }
            }
            if !message.isOwn { Spacer(minLength: WontonTheme.spacing.s48) }
        }
    }

    @ViewBuilder private func menuItem(_ action: MessageAction) -> some View {
        switch action {
        case let .copy(text):
            Button {
                UIPasteboard.general.string = text
            } label: {
                Label { Text(.messageCopy) } icon: { WontonIcon.copy.image }
            }
        case let .reply(quoted):
            Button {
                reply(quoted)
            } label: {
                Label { Text(.messageReply) } icon: { WontonIcon.reply.image }
            }
            .accessibilityIdentifier("messageReply")
        case .forward:
            Button {
                forward()
            } label: {
                Label { Text(.messageForward) } icon: { WontonIcon.forward.image }
            }
            .accessibilityIdentifier("messageForward")
        case .redact:
            Button(role: .destructive) {
                redact()
            } label: {
                Label { Text(.messageRedact) } icon: { WontonIcon.trash.image }
            }
            .accessibilityIdentifier("messageRedact")
        // 核心和 App 分开升级时，不认识的菜单项不显示。
        default:
            EmptyView()
        }
    }

    @ViewBuilder private var bubble: some View {
        let body = quoted
            .padding(.horizontal, WontonTheme.spacing.s12)
            .padding(.vertical, WontonTheme.spacing.s8)
        if message.isOwn {
            body.accentFill(shape)
        } else {
            body.background(shape.fill(Color(colors.surfaceSubtle)))
        }
    }

    /// 回复别人时，正文上方先画被引用的原消息（ADR-0073）；点引用跳转还没做。
    @ViewBuilder private var quoted: some View {
        if let quote = message.reply {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                ReplyQuoteView(quote: quote, core: core, ink: textColor)
                laidOut
            }
        } else {
            laidOut
        }
    }

    @ViewBuilder private var laidOut: some View {
        if let reserve = footerReserve {
            // 正文末尾有一段和时间等宽的透明占位，时间压在它上面。放不下时占位自己换行，时间就落在新的一行。
            ZStack(alignment: Alignment(horizontal: .trailing, vertical: .lastTextBaseline)) {
                styled(reserve: reserve)
                // 占位文字已经念过一遍时间了。
                footer.accessibilityHidden(true)
            }
        } else {
            // Grid：气泡和最宽的一行一样宽，正文靠左、时间靠右。VStack 做不到，撑满又会把短消息的气泡也撑满。
            Grid(alignment: .leading, verticalSpacing: WontonTheme.spacing.s4) {
                styled(reserve: nil)
                footer.gridCellAnchor(.trailing)
            }
        }
    }

    private func styled(reserve: String?) -> some View {
        content(reserve: reserve)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(textColor)
    }

    /// 时间能压在最后一行上时，给它留的透明文字。发送失败的说明太长，文件卡片和代码块、表格后面也没有文字行，都单独占一行。
    /// 开头是普通空格：不能和前一个词连着折行。
    private var footerReserve: String? {
        guard case let .text(_, blocks, files) = message.body, files.isEmpty, let last = blocks.last else { return nil }
        switch last.kind {
        case .paragraph, .heading, .listItem: break
        case .code, .table, .rule: return nil
        }
        if case .failed = message.sendState { return nil }
        var parts = [TimelineRow.time(message.timestampMs)]
        if message.edited { parts.insert(String(localized: .messageEdited), at: 0) }
        if case .sending = message.sendState { parts.append(String(localized: .messageSending)) }
        if case .sent = message.sendState, message.isOwn, !message.readBy.isEmpty {
            parts.append(String(localized: .messageRead))
        }
        let glued = parts.joined(separator: "  ").replacingOccurrences(of: " ", with: "\u{00A0}")
        return " \u{00A0}\u{00A0}" + glued
    }

    private var textColor: Color {
        Color(message.isOwn ? colors.textOnAccentFill : colors.textPrimary)
    }

    @ViewBuilder private func content(reserve: String?) -> some View {
        switch message.body {
        case let .text(_, blocks, files):
            TextMessageBody(blocks: blocks, ink: textColor, files: files, trailingReserve: reserve)
        case let .image(image):
            MediaImage(image: image, core: core, sendState: message.sendState)
        case let .confirmation(view):
            ConfirmationCardView(
                view: view,
                answer: {
                    cardAction(
                        .answerConfirmation(
                            id: view.card.id,
                            answer: $0.answer,
                            choiceId: $0.choiceId,
                            secrets: $1,
                            settings: [:]
                        )
                    )
                },
                stopWindow: { cardAction(.stopConfirmationWindow(id: view.card.id)) }
            )
        case .undecryptable:
            Label {
                Text(.messageUndecryptable)
            } icon: {
                WontonIcon.lockKey.view(size: WontonTheme.size.iconSmall)
            }
            .wontonText(WontonTheme.typography.supporting)
        case .redacted:
            Text(.messageRedacted).italic().opacity(WontonTheme.ratio.secondaryContent)
        case let .unsupported(kind):
            Text(.messageUnsupported(kind))
                .wontonText(WontonTheme.typography.supporting)
                .opacity(WontonTheme.ratio.secondaryContent)
        }
    }

    private var footer: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            if message.edited { Text(.messageEdited) }
            Text(TimelineRow.time(message.timestampMs))
            switch message.sendState {
            case .sending:
                Text(.messageSending)
            case let .failed(reason, recoverable):
                Button {
                    isChoosingFailed = true
                } label: {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        WontonIcon.warningCircle.view(size: WontonTheme.size.iconTiny)
                        if recoverable {
                            Text(.messageSendFailedRetrying)
                        } else {
                            Text(.messageSendFailed(reason))
                        }
                    }
                    .foregroundStyle(Color(colors.danger))
                }
                .buttonStyle(.plain)
                .wontonActionSheet(isPresented: $isChoosingFailed, actions: [
                    WontonSheetAction(title: String(localized: .messageResend), icon: .refresh, perform: retry),
                    WontonSheetAction(
                        title: String(localized: .commonDelete), icon: .trash, isDanger: true, perform: cancel
                    )
                ])
            case .sent:
                if message.isOwn, !message.readBy.isEmpty { Text(.messageRead) }
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
        .accessibilityLabel(Text(.messagePhoto))
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
                    WontonNotice(
                        icon: .imageBroken, title: String(localized: .messageOriginalUnavailable),
                        lines: [failure.message]
                    )
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
            .toolbar { Button(.commonDone) { dismiss() } }
        }
        .task { original = await MediaFailure.load(image.media, from: core) }
    }
}

/// 媒体取不到的原因，引用缩略图和原图共用。
struct MediaFailure: Error {
    let message: String

    static func load(_ media: MediaHandle, from core: ClientCore) async -> Result<UIImage, MediaFailure> {
        do {
            let data = try await core.loadMedia(media: media)
            guard let image = UIImage(data: data) else {
                return .failure(MediaFailure(message: String(localized: .messageImageFormatUnsupported)))
            }
            return .success(image)
        } catch {
            return .failure(MediaFailure(message: error.problem.title))
        }
    }
}
