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
    /// 点“编辑”：参数是原文，壳先填进输入框再发 `StartEdit`（ADR-0109 第 10 节）。
    let edit: (String) -> Void
    let forward: () -> Void
    let redact: () -> Void
    /// Agent 在等回答的卡片就在这个房间：滚到那张卡片（ADR-0082）。
    var openCard: (String) -> Void = { _ in }
    /// 回答 Agent 自己的提问或消息下的选项按钮（`choices` 的 key），以主人的身份发反应。
    var answerChoice: (String) -> Void = { _ in }
    /// 点了气泡里的引用：跳到原消息（参数是原消息的事件 ID，ADR-0109 第 4 节）。
    var jump: (String) -> Void = { _ in }
    /// 核心正在为这一条的引用往前翻页找原消息：引用上画加载。
    var isJumping = false
    /// 群聊里别人的组尾旁边画的头像；私聊不给。
    var avatar: ((TimelineMessage) -> AnyView)?
    /// 这一条属于哪一组照片，点开查看器从这一张起翻；单张或不成组时 nil。
    var photoGroup: PhotoGroup?
    /// 进旁聊（参数是根的事件 ID）：旁聊卡片和结果行都用它（ADR-0093）。
    var onOpenSideChat: (String) -> Void = { _ in }
    @Environment(\.wontonColors) private var colors
    @Environment(\.chatPalette) private var palette

    var body: some View {
        switch item.content {
        case let .message(message):
            // 群聊里名字只在组首写一次；组内位置由核心给（ADR-0109）。
            MessageBubble(
                itemId: item.id, message: message, core: core,
                showsSender: showsSender && message.run.startsRun,
                flushesPrevious: flushesPrevious, flushesNext: flushesNext,
                retry: retry, cancel: cancel,
                cardAction: cardAction, reply: reply, edit: edit, forward: forward, redact: redact,
                openSideChat: onOpenSideChat, answerChoice: answerChoice,
                jump: jump, isJumping: isJumping, avatar: message.isOwn ? nil : avatar,
                photoGroup: photoGroup
            )
        case let .notice(text):
            note(Text(text))
        case let .agentRun(
            _, state, step, steps, progress, result, end, body, cardId, choices, senderName, timestampMs,
            _):
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
            // 跨天处居中的日期胶囊（ADR-0109 第 4 节）；几点在每条气泡里。
            DatePill(timestampMs: timestampMs).frame(maxWidth: .infinity)
        case .readMarker:
            HStack(spacing: WontonTheme.spacing.s8) {
                edge
                Text(.Chat.timelineReadMarker)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textPrimary))
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

    /// 说明行和日期胶囊一个样：压在聊天背景上的半透明胶囊（ADR-0109 第 9 节）。
    private func note(_ text: Text) -> some View {
        text
            .wontonText(WontonTheme.typography.datePill)
            .foregroundStyle(palette.serviceInk)
            .multilineTextAlignment(.center)
            .padding(.horizontal, WontonTheme.spacing.s8)
            .padding(.vertical, WontonTheme.spacing.s4)
            .background(palette.serviceFill, in: RoundedRectangle(cornerRadius: WontonTheme.radii.bubbleJoined + 3))
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
    let itemId: String
    let message: TimelineMessage
    let core: ClientCore
    let showsSender: Bool
    var flushesPrevious: Bool = false
    var flushesNext: Bool = false
    let retry: () -> Void
    let cancel: () -> Void
    let cardAction: (Action) -> Void
    let reply: (QuotedMessage) -> Void
    let edit: (String) -> Void
    let forward: () -> Void
    let redact: () -> Void
    /// 挂在这条消息上的旁聊卡片点了就进那个旁聊（ADR-0093）。
    let openSideChat: (String) -> Void
    let answerChoice: (String) -> Void
    let jump: (String) -> Void
    let isJumping: Bool
    /// 群聊里别人的消息才有：组尾画头像，其余几条留出同样宽的空位，一组的气泡才对得齐。
    let avatar: ((TimelineMessage) -> AnyView)?
    let photoGroup: PhotoGroup?
    @State private var isChoosingFailed = false
    @Environment(\.wontonColors) private var colors
    @Environment(\.chatPalette) private var palette

    /// 长按菜单底部的发送时间：完整日期加时间，隔了天也对得上（Telegram 长按同样显示）。
    private var sentAt: String {
        TimelineRow.date(message.timestampMs).formatted(.dateTime.year().month().day().hour().minute())
    }

    private var shape: MessageBubbleShape {
        MessageBubbleShape(run: message.run, isOwn: message.isOwn)
    }

    var body: some View {
        VStack(alignment: message.isOwn ? .trailing : .leading, spacing: WontonTheme.spacing.s4) {
            HStack(alignment: .bubbleBottom, spacing: 0) {
                if message.isOwn {
                    Spacer(minLength: 0)
                    failedBadge
                } else if let avatar {
                    Group {
                        if message.run.endsRun {
                            avatar(message)
                                .accessibilityElement(children: .ignore)
                                .accessibilityLabel(message.senderName)
                                .accessibilityIdentifier("messageAvatar")
                        } else {
                            Color.clear
                        }
                    }
                    .frame(width: WontonTheme.size.messageAvatar, height: WontonTheme.size.messageAvatar)
                    .padding(.trailing, WontonTheme.spacing.s4)
                }
                BubbleColumn {
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
                    if let choices = message.choices {
                        // 按钮对齐气泡的身子，不占尾巴那条边。
                        MessageChoicesView(choices: choices, answer: answerChoice)
                            .padding(message.isOwn ? .trailing : .leading, WontonTheme.size.bubbleTail)
                    }
                }
                // 优先级高于两边的空白：不然 HStack 先把宽度平分给空白，气泡的宽度上限就按一半算了。
                .layoutPriority(1)
                if !message.isOwn { Spacer(minLength: 0) }
            }
            // 有回复的普通消息下面挂着旁聊的卡片（ADR-0093 §6）。
            if let card = message.sideChat {
                SideChatCardView(card: card) { openSideChat(card.rootEventId) }
                    .padding(.leading, avatar == nil ? 0 : WontonTheme.size.messageAvatar + WontonTheme.spacing.s4)
            }
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
        case let .edit(text):
            Button {
                edit(text)
            } label: {
                Label { Text(.Message0.messageEdit) } icon: { Image(systemName: "pencil") }
            }
            .accessibilityIdentifier("messageEdit")
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
            .padding(.leading, WontonTheme.spacing.bubbleInsetX + (message.isOwn ? 0 : tail))
            .padding(.trailing, WontonTheme.spacing.bubbleInsetX + (message.isOwn ? tail : 0))
            // 同一次发送里贴着的照片不留上下衬垫，贴成一摞（ADR-0094）。
            .padding(.top, flushesPrevious ? 0 : WontonTheme.spacing.bubbleInsetY)
            .padding(.bottom, flushesNext ? 0 : WontonTheme.spacing.bubbleInsetY)
        // 气泡是内容：纯色、不透明，不描边、不叠颗粒，也不铺玻璃（ADR-0107、ADR-0109 第 9 节）；靠投影浮在背景上（ADR-0130）。
        body.background(shape.fill(message.isOwn ? Color(colors.primary) : palette.incoming).chatLift(palette))
            // 整只气泡是一个容器：走查按条目定位，里面的引用、正文各自照旧查得到。
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("message.\(itemId)")
    }

    /// 群聊里组首气泡顶部写发送者的名字；回复别人时，正文上方画被引用的原消息（ADR-0073），点它跳到原消息。
    /// 带选项的气泡底下垫一行看不见的字，至少和按钮一样宽。
    private var quoted: some View {
        ZStack(alignment: .topLeading) {
            if let choices = message.choices { ChoicesWidthReserve(choices: choices) }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                if showsSender, !message.isOwn {
                    Text(message.senderName)
                        .wontonText(WontonTheme.typography.senderName)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                }
                if let quote = message.reply {
                    Button {
                        jump(quote.eventId)
                    } label: {
                        ReplyQuoteView(
                            quote: quote, core: core, ink: textColor,
                            emphasis: Color(message.isOwn ? colors.onPrimary : colors.textPrimary),
                            isLoading: isJumping
                        )
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(isJumping)
                    .accessibilityIdentifier("messageQuote.\(itemId)")
                    .padding(.bottom, WontonTheme.spacing.s4)
                }
                laidOut
            }
        }
        // 气泡按内容的高度排，不接受别人给的高度：给矮了正文会被截成“…”，给高了里面能伸缩的东西会被撑开
        // （2026-11-20 真机上长消息每段被截断、引用块撑到九行高）。
        .fixedSize(horizontal: false, vertical: true)
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
            Grid(alignment: .leading, verticalSpacing: WontonTheme.spacing.s4 / 2) {
                styled(reserve: nil)
                MessageMeta(message: message).gridCellAnchor(.trailing)
            }
        }
    }

    private func styled(reserve: String?) -> some View {
        content(reserve: reserve)
            .wontonText(WontonTheme.typography.message)
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

    /// 发送失败：气泡旁的红色感叹号，点它重发或删除。读屏讲类别，核心还分不出类别（`other`）时讲原文；
    /// 原文写在点开的面板标题上。
    @ViewBuilder private var failedBadge: some View {
        if case let .failed(failure, details, recoverable) = message.sendState {
            // 气泡旁和读屏只留一句话；核心还分不出类别（`other`）时用原文；SDK 的原文收在面板标题上（ADR-0109 第 10 节）。
            let label = if recoverable {
                String(localized: .Message2.messageSendFailedRetrying)
            } else if failure == .other, let details {
                details
            } else {
                String(localized: failure.text)
            }
            Button {
                isChoosingFailed = true
            } label: {
                WontonIcon.warningCircleFill.view(size: WontonTheme.size.icon)
                    .foregroundStyle(Color(colors.danger))
                    .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            .accessibilityIdentifier("messageFailed")
            .wontonActionSheet(isPresented: $isChoosingFailed, title: details, actions: [
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
        Color(message.isOwn ? colors.onPrimary : colors.textPrimary)
    }
}

/// 气泡里的正文，按消息的种类画。
private extension MessageBubble {
    @ViewBuilder func content(reserve: String?) -> some View {
        switch message.body {
        case let .text(_, blocks, files):
            TextMessageBody(blocks: blocks, ink: textColor, files: files, trailingReserve: reserve)
        case let .image(image):
            // 一次发送里主人写的字是最后一张照片的说明文字（ADR-0094），画在照片下面。
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                MediaImage(image: image, group: photoGroup, core: core, sendState: message.sendState)
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
