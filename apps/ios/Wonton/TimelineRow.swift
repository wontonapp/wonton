import SwiftUI
import WontonClient

struct TimelineRow: View {
    let item: TimelineItem
    let core: ClientCore
    /// 私聊里对方是谁看标题就知道，不在每条消息上重复名字。
    let showsSender: Bool
    /// 和上一条、下一条是不是同一次发送里贴着的两张照片（ADR-0094）：贴成一摞，中间不留衬垫。
    var flushesPrevious: Bool = false
    var flushesNext: Bool = false
    /// 重发、删除发送失败的这一条。
    let retry: () -> Void
    let cancel: () -> Void
    /// 回答这张确认卡片。
    let cardAction: (Action) -> Void
    /// 进入回复状态，引用由核心随菜单给。
    let reply: (QuotedMessage) -> Void
    let forward: () -> Void
    let redact: () -> Void
    /// Agent 在等回答的卡片就在这个房间：滚到那张卡片（ADR-0082）。
    var openCard: (String) -> Void = { _ in }
    /// 回答 Agent 自己的提问（`choices` 的 key），以主人的身份发反应。
    var answerChoice: (String) -> Void = { _ in }
    /// 进旁聊（参数是根的事件 ID）：旁聊卡片和结果行都用它（ADR-0093）。
    var onOpenSideChat: (String) -> Void = { _ in }
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch item.content {
        case let .message(message):
            // 群聊里名字只在组首写一次；组内位置由核心给（ADR-0109）。
            MessageBubble(
                message: message, core: core, showsSender: showsSender && message.run.startsRun,
                flushesPrevious: flushesPrevious, flushesNext: flushesNext,
                retry: retry, cancel: cancel,
                cardAction: cardAction, reply: reply, forward: forward, redact: redact,
                openSideChat: onOpenSideChat
            )
        case let .notice(text):
            note(Text(text))
        case let .agentRun(
            _, state, step, steps, progress, result, end, body, cardId, choices, senderName, timestampMs):
            AgentRunRow(
                state: state,
                step: step,
                steps: steps,
                progress: progress,
                result: result,
                end: end,
                bodyText: body,
                cardId: cardId,
                choices: choices,
                senderName: senderName,
                timestampMs: timestampMs,
                openCard: openCard,
                answerChoice: answerChoice
            )
        case let .profileChange(_, name, previousName, _):
            if let previousName {
                note(Text(.Chat.timelineRenamed(previousName, name)))
            } else {
                note(Text(.Chat.timelineAvatarChanged(name)))
            }
        case let .dateDivider(timestampMs):
            // 分隔线带着这一天第一条消息的时间：日期和时间写在一行，那条消息上方不再另写。
            note(Text(Self.date(timestampMs), format: .dateTime.year().month().day().weekday().hour().minute()))
        case .readMarker:
            HStack(spacing: WontonTheme.spacing.s8) {
                edge
                Text(.Chat.timelineReadMarker)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.accentInk))
                edge
            }
        case let .sideChat(card):
            // 旁聊根消息（ADR-0093）：画成卡片，点了进那个旁聊。
            SideChatCardView(card: card) { onOpenSideChat(card.rootEventId) }
        case let .sideChatResult(rootEventId, title, state):
            SideChatResultLine(title: title, state: state) { onOpenSideChat(rootEventId) }
        case .timelineStart:
            note(Text(.Chat.timelineStart))
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
    var flushesPrevious: Bool = false
    var flushesNext: Bool = false
    let retry: () -> Void
    let cancel: () -> Void
    let cardAction: (Action) -> Void
    let reply: (QuotedMessage) -> Void
    let forward: () -> Void
    let redact: () -> Void
    /// 挂在这条消息上的旁聊卡片点了就进那个旁聊（ADR-0093）。
    let openSideChat: (String) -> Void
    @State private var isChoosingFailed = false
    @Environment(\.wontonColors) private var colors

    /// 长按菜单底部的发送时间：完整日期加时间，隔了天也对得上（Telegram 长按同样显示）。
    private var sentAt: String {
        TimelineRow.date(message.timestampMs).formatted(.dateTime.year().month().day().hour().minute())
    }

    private var shape: MessageBubbleShape {
        MessageBubbleShape(run: message.run, isOwn: message.isOwn)
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if message.isOwn {
                Spacer(minLength: WontonTheme.spacing.s48)
                failedBadge
            }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                if showsSender, !message.isOwn {
                    Text(message.senderName)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                bubble
                    // 系统的菜单自带预览、背景模糊和材质（ADR-0029）。菜单项的图标也用系统的 SF Symbols：
                    // 大小、粗细由系统跟着菜单的字排。
                    .contentShape(.contextMenuPreview, shape)
                    .contextMenu {
                        ForEach(Array(message.actions.enumerated()), id: \.offset) { _, action in
                            menuItem(action)
                        }
                        Section {
                            Text(.Message1.messageSentAt(sentAt))
                                .accessibilityIdentifier("messageSentAt")
                        }
                    }
                // 有回复的普通消息下面挂着旁聊的卡片（ADR-0093 §6）。
                if let card = message.sideChat {
                    SideChatCardView(card: card) { openSideChat(card.rootEventId) }
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
                Label { Text(.Message2.messageCopy) } icon: { Image(systemName: "doc.on.doc") }
            }
        case let .reply(quoted):
            Button {
                reply(quoted)
            } label: {
                Label { Text(.Message0.messageReply) } icon: { Image(systemName: "arrowshape.turn.up.left") }
            }
            .accessibilityIdentifier("messageReply")
        case .forward:
            Button {
                forward()
            } label: {
                Label { Text(.Message1.messageForward) } icon: { Image(systemName: "arrowshape.turn.up.right") }
            }
            .accessibilityIdentifier("messageForward")
        case .redact:
            // 隔开前面的项，别和转发挨着误点。管理员也能撤别人的消息，写明是谁的，免得以为撤的是自己的。
            Divider()
            Button(role: .destructive) {
                redact()
            } label: {
                Label {
                    message.isOwn
                        ? Text(.Message1.messageRedact)
                        : Text(.Message2.messageRedactOthers(message.senderName))
                } icon: {
                    Image(systemName: "trash")
                }
            }
            .accessibilityIdentifier("messageRedact")
        // 核心和 App 分开升级时，不认识的菜单项不显示。
        default:
            EmptyView()
        }
    }

    @ViewBuilder private var bubble: some View {
        // 发送者一侧让出尾巴那条边（见 MessageBubbleShape）。
        let tail = WontonTheme.size.bubbleTail
        let body = quoted
            .padding(.leading, WontonTheme.spacing.s12 + (message.isOwn ? 0 : tail))
            .padding(.trailing, WontonTheme.spacing.s12 + (message.isOwn ? tail : 0))
            // 同一次发送里贴着的照片不留上下衬垫，贴成一摞（ADR-0094）。
            .padding(.top, flushesPrevious ? 0 : WontonTheme.spacing.s8)
            .padding(.bottom, flushesNext ? 0 : WontonTheme.spacing.s8)
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

    /// 时间常驻气泡右下角（ADR-0109 第 3 节）：放得下和末行同排，放不下另起一行；纯图片压在图片右下角。
    @ViewBuilder private var laidOut: some View {
        if isBareImage {
            styled(reserve: nil)
        } else if let reserve = metaReserve {
            // 正文末尾有一段和这一行等宽的透明占位，这一行压在它上面。放不下时占位自己换行，这一行就落在新的一行。
            ZStack(alignment: Alignment(horizontal: .trailing, vertical: .lastTextBaseline)) {
                styled(reserve: reserve)
                MessageMeta(message: message, textIsSpoken: true)
            }
        } else {
            // Grid：气泡和最宽的一行一样宽，正文靠左、时间靠右。VStack 做不到，撑满又会把短消息的气泡也撑满。
            Grid(alignment: .leading, verticalSpacing: WontonTheme.spacing.s4) {
                styled(reserve: nil)
                MessageMeta(message: message).gridCellAnchor(.trailing)
            }
        }
    }

    private func styled(reserve: String?) -> some View {
        content(reserve: reserve)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(textColor)
    }

    /// 没有说明文字的图片：时间画在图片上。
    private var isBareImage: Bool {
        if case let .image(image) = message.body { return image.caption == nil }
        return false
    }

    /// 时间能压在最后一行上时，给它留的透明文字。文件卡片和代码块、表格后面没有文字行，时间单独占一行。
    private var metaReserve: String? {
        guard case let .text(_, blocks, files) = message.body, files.isEmpty, let last = blocks.last else { return nil }
        switch last.kind {
        case .paragraph, .heading, .listItem: break
        case .code, .table, .rule: return nil
        }
        return MessageMeta.reserve(for: message)
    }

    /// 发送失败：气泡旁的红色感叹号，点它重发或删除；原因写在面板的标题上。
    @ViewBuilder private var failedBadge: some View {
        if case let .failed(reason, recoverable) = message.sendState {
            let title = recoverable
                ? String(localized: .Message2.messageSendFailedRetrying)
                : String(localized: .Message1.messageSendFailed(reason))
            Button {
                isChoosingFailed = true
            } label: {
                WontonIcon.warningCircleFill.view(size: WontonTheme.size.icon)
                    .foregroundStyle(Color(colors.danger))
                    .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityIdentifier("messageFailed")
            .wontonActionSheet(isPresented: $isChoosingFailed, title: title, actions: [
                WontonSheetAction(
                    title: String(localized: .Message1.messageResend), icon: .refresh, perform: retry
                ),
                WontonSheetAction(
                    title: String(localized: .Common.commonDelete), icon: .trash, isDanger: true, perform: cancel
                )
            ])
        }
    }

    private var textColor: Color {
        Color(message.isOwn ? colors.textOnAccentFill : colors.textPrimary)
    }

    @ViewBuilder private func content(reserve: String?) -> some View {
        switch message.body {
        case let .text(_, blocks, files):
            TextMessageBody(blocks: blocks, ink: textColor, files: files, trailingReserve: reserve)
        case let .image(image):
            // 一次发送里主人写的字是最后一张照片的说明文字（ADR-0094），画在照片下面。
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                MediaImage(image: image, core: core, sendState: message.sendState)
                    .overlay(alignment: .bottomTrailing) {
                        if image.caption == nil {
                            MessageMeta(message: message, onMedia: true).padding(WontonTheme.spacing.s4)
                        }
                    }
                if let caption = image.caption {
                    TextMessageBody(blocks: caption.blocks, ink: textColor, files: [])
                }
            }
        case let .file(file):
            // 一次发送里主人写的字是说明文字（ADR-0094），和图片一样画在卡片下面的气泡墨色里。
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                // token 变了就是另一份媒体：换掉视图身份，状态（已下载路径、错误）跟着重来，不会显示新名打开旧内容。
                MessageFileBody(file: file, core: core).id(file.media.token)
                if let caption = file.caption {
                    TextMessageBody(blocks: caption.blocks, ink: textColor, files: [])
                }
            }
        case let .confirmation(view):
            ConfirmationCardView(
                view: view,
                answer: {
                    cardAction(
                        .chat(action: .answerConfirmation(
                            id: view.card.id,
                            answer: $0.answer,
                            choiceId: $0.choiceId,
                            secrets: $1,
                            settings: $2,
                            devices: $3
                        ))
                    )
                },
                stopWindow: { cardAction(.chat(action: .stopConfirmationWindow(id: view.card.id))) }
            )
        case .undecryptable:
            Label {
                Text(.Message1.messageUndecryptable)
            } icon: {
                WontonIcon.lockKey.view(size: WontonTheme.size.iconSmall)
            }
            .wontonText(WontonTheme.typography.supporting)
        case .redacted:
            Text(.Message2.messageRedacted).italic().opacity(WontonTheme.ratio.secondaryContent)
        case let .unsupported(kind):
            Text(.Message1.messageUnsupported(kind))
                .wontonText(WontonTheme.typography.supporting)
                .opacity(WontonTheme.ratio.secondaryContent)
        }
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
        .accessibilityLabel(Text(.Message2.messagePhoto))
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
                        icon: .imageBroken, title: String(localized: .Message1.messageOriginalUnavailable),
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
            .toolbar { Button(.Common.commonDone) { dismiss() } }
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
                return .failure(MediaFailure(message: String(localized: .Message0.messageImageFormatUnsupported)))
            }
            return .success(image)
        } catch {
            return .failure(MediaFailure(message: error.problem.title))
        }
    }
}
