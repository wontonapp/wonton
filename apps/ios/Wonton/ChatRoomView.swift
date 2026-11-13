import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let plugins: PluginsModel
    let roomId: String
    /// 旁聊根的事件 ID；空是房间的主时间线（ADR-0093）。旁聊页盖在主对话上（全屏盖层），主对话继续订阅。
    let threadRoot: String?
    let title: String
    @State private var timeline: TimelineModel
    @State private var composer: ComposerModel
    @State private var presenter: RoomPresenter
    @State private var pin = BottomPin()
    @State private var isKeyboardVisible = false
    @State private var pageAnchor: String?
    /// 正在选转发目标的条目、正在确认撤回的条目。
    @State private var forwarding: String?
    @State private var pickingForwardRoom = false
    @State private var redactItem: String?
    @State private var showingRedact = false
    /// 进房间的落点摆好之前不标已读（ADR-0071）：标早了已读行会跳到最新一条，落点就丢了。
    @State private var placed = false
    /// 在最新一条之后往上推，拉出房间的面板（ADR-0052）。
    @State private var roomPull = PullAnchor(attachesToAncestor: false, edge: .bottom)
    @Environment(OnboardingModel.self) private var onboarding
    @Environment(PagesModel.self) private var pages

    init(model: ChatModel, plugins: PluginsModel, roomId: String, threadRoot: String? = nil, title: String) {
        self.model = model
        self.plugins = plugins
        self.roomId = roomId
        self.threadRoot = threadRoot
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId, threadRoot: threadRoot))
        _composer = State(initialValue: ComposerModel(chat: model, roomId: roomId, threadRoot: threadRoot))
        _presenter = State(initialValue: RoomPresenter(files: model.files))
    }

    var body: some View {
        if threadRoot == nil {
            // 旁聊页全屏盖在这条主对话上（ADR-0093）：盖层的机制在 SideChatCover。
            SideChatCover(model: model, plugins: plugins, roomId: roomId, title: title) {
                RoomPanelHost(roomId: roomId, panel: timeline.panel, selected: timeline.panelSelected,
                              blocked: entryBlocked, pullAnchor: roomPull) {
                    roomContent
                        .overlay(alignment: .topLeading) { ScrollDebug(text: pin.snapshot) }
                }
            }
        } else {
            // 旁聊页没有房间的面板（ADR-0093）：不挂面板宿主，也没有上推入口。
            roomContent
        }
    }
}

@MainActor
private extension ChatRoomView {
    /// 房间里另有东西开着：长按菜单之外的转发、删除确认不响应上推入口（ADR-0052）。
    private var entryBlocked: Bool { pickingForwardRoom || showingRedact }

    private var roomContent: some View {
        VStack(spacing: 0) {
            // 旁聊页顶栏下面是它的状态；主对话顶上是“这个 AI 不支持旁聊”的提示（ADR-0093）。
            if threadRoot != nil {
                if let state = timeline.sideChat?.state { SideChatHeaderLine(state: state) }
            } else if model.noSideChatsRoom == roomId {
                NoSideChatsHint()
            }
            if let failure = timeline.failure {
                ProblemView(problem: failure) { timeline.attempt += 1 }
                    .frame(maxHeight: .infinity)
            } else {
                messages
            }
            if let notice = timeline.notice {
                KeyNoticeBanner(notice: notice)
            }
            TransfersStrip(roomId: roomId)
            if !isReadOnly {
                if threadRoot == nil, let bot = onboarding.viewModel.primaryBotUserId,
                   model.viewModel.rooms.first(where: { $0.roomId == roomId })?.directUserId == bot,
                   !onboarding.viewModel.chatPrompts.isEmpty {
                    OnboardingChatPrompts(prompts: onboarding.viewModel.chatPrompts) {
                        model.appendDraft($0, to: target)
                    }
                }
                ComposerView(model: model, composer: composer, pin: pin)
            }
        }
        .modifier(RoomPresentations(presenter: presenter))
        .wontonSheet(isPresented: $pickingForwardRoom,
            title: String(localized: .Message0.messageForwardTitle)) { close in
            ForwardRoomSheet(model: model, plugins: plugins, rooms: forwardRooms) { room in
                close {
                    guard let item = forwarding else { return }
                    model.forward(item, from: roomId, to: room.roomId)
                    forwarding = nil
                }
            }
        }
        .wontonDialog(
            redactTitle,
            message: String(localized: .Message2.messageRedactMessage),
            isPresented: $showingRedact,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)),
                WontonDialogAction(title: String(localized: .Message1.messageRedact), kind: .danger) {
                    if let item = redactItem { sendTimeline(.redactMessage(target: target, itemId: item)) }
                }
            ],
            afterDismiss: { redactItem = nil }
        )
        .environment(model.files)
        .environment(presenter)
        // 开新旁聊的面板（ADR-0093 §6）。
        .modifier(NewSideChatSheet(model: model, roomId: roomId))
        // 盖层见 body 里的 SideChatCover：这里不另做系统呈现，免得和收起的“开新旁聊”面板撞在同一个呈现时机上。
        .modifier(RoomTopBar(
            title: bot?.name ?? title,
            avatar: barAvatar,
            sideChatTitle: threadRoot == nil ? nil : timeline.sideChat?.title ?? String(localized: .Chat.sideChat),
            close: threadRoot == nil ? nil : { model.closeSideChat(in: roomId) }
        ) {
            if supportsSideChats {
                WontonIconButton(icon: .compose, label: String(localized: .Chat.sideChatNew)) {
                    model.composingSideChat = roomId
                }
            } else {
                EmptyView()
            }
        })
        // 入口按钮压进这一页。房间也能从别的标签页打开，那边的导航栈不认聊天页的路由，所以在这里登记一次。
        .navigationDestination(for: ChatRoute.self) { route in
            if case .chatBackup = route { ChatBackupView(model: model) }
        }
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .onChange(of: entryBlocked, initial: true) { _, blocked in
            // 旁聊页没有面板，不报（ADR-0093）。
            if threadRoot == nil {
                model.send(.setRoomEntryBlocked(roomId: roomId, blocked: blocked))
            }
        }
        .task(id: model.draftKey(target)) { await model.loadDraft(target) }
        .task(id: model.draftKey(target)) { await composer.observe() }
        .task(id: timeline.attempt) {
            if await timeline.observe(pin: pin) {
                if threadRoot != nil {
                    // 旁聊没了：只收起旁聊，主对话还在。
                    model.closeSideChat(in: roomId)
                } else {
                    model.path = []
                    model.visiting = nil
                    pages.open(.chat)
                }
            }
        }
        .onDisappear {
            if threadRoot == nil {
                // 房间关了，开着的旁聊跟着关（换房间、路径被换掉时没有别的收口）。
                model.sideChats[roomId] = nil
                if model.noSideChatsRoom == roomId {
                    model.noSideChatsRoom = nil
                }
            } else if model.sideChats[roomId] == threadRoot {
                // 旁聊页收起时同步关掉：改回主对话就不再算“旁聊开着”。
                model.closeSideChat(in: roomId)
            }
        }
        .onChange(of: covered) { _, isCovered in
            // 盖住拿开时重新报告一次方向：还停着底部，就把盖住时攒下的消息读掉（ADR-0093）。
            if placed, pin.isPinned, !isCovered { reportPosition(true) }
        }
    }

    /// 这条时间线：主对话或一个旁聊（ADR-0093）。
    private var target: TimelineTarget { TimelineTarget(roomId: roomId, threadRoot: threadRoot) }
}

/// 消息列表、落点摆放和位置报告（ADR-0071）。
@MainActor
private extension ChatRoomView {
    private var messages: some View {
        ScrollViewReader { scroller in
            // 不用 ScrollView + LazyVStack：行高相差悬殊时（一行和六十行）它的内容高度全靠估算，同样的数据几毫秒内差出几千点，
            // 滚动位置落在没有画任何行的地方，整屏空白（iOS 26.5 模拟器实测）。List 量过的行高不再变。
            List {
                // 这一行常驻，只换内容：List 的结构变了（条件行增删）会把滚动位置重置到顶上（iOS 26.5 模拟器实测）。
                Group {
                    if timeline.backPagination != .reachedStart {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                            // 滚到顶上时（这一行出现）加载更早的消息；一页加载完，这一行还在就再加载一页。
                            .task(id: PageRequest(timeline)) {
                                // 第一批消息到时这一行闪现一下，紧接着就滚到底了：停得住才算翻到了顶，滚走时这里被取消。
                                guard (try? await Task.sleep(for: .milliseconds(300))) != nil else { return }
                                if timeline.isLoaded, timeline.backPagination == .idle {
                                    pageAnchor = oldestEvent
                                    sendTimeline(.paginateBackwards(target: target))
                                }
                            }
                            .onDisappear { pageAnchor = nil }
                    } else {
                        Color.clear.frame(height: 1)
                    }
                }
                // 进房间时列表从头排起，这一行一定会出现。
                .background(EnclosingScrollView { scrollView in
                    pin.attach(scrollView)
                    roomPull.attach(to: scrollView)
                })
                .timelineListRow()
                ForEach(Array(timeline.items.enumerated()), id: \.element.id) { index, item in
                    let joinsPrevious = index > 0 && item.joins(timeline.items[index - 1])
                    let joinsNext = index + 1 < timeline.items.count && item.joins(timeline.items[index + 1])
                    TimelineRow(
                        item: item, core: model.core, showsSender: !isDirect,
                        joinsPrevious: joinsPrevious, joinsNext: joinsNext,
                        retry: { sendTimeline(.retrySend(target: target, itemId: item.id)) },
                        cancel: { sendTimeline(.cancelSend(target: target, itemId: item.id)) },
                        cardAction: { model.send($0) },
                        reply: { model.startReply(.ready(message: $0), in: target) },
                        forward: {
                            forwarding = item.id
                            pickingForwardRoom = true
                        },
                        redact: {
                            redactItem = item.id
                            showingRedact = true
                        },
                        openCard: { cardId in
                            guard let target = timeline.items.first(where: { $0.cardId == cardId }) else { return }
                            pin.unpin()
                            DispatchQueue.main.async { scroller.scrollTo(target.id, anchor: .center) }
                        },
                        answerChoice: { key in
                            model.send(.answerAgentChoice(roomId: roomId, itemId: item.id, key: key))
                        },
                        onOpenSideChat: { model.openSideChat($0, in: roomId) }
                    )
                    .id(item.id)
                    .timelineListRow(joinsPrevious: joinsPrevious, joinsNext: joinsNext)
                    .modifier(ReplySwipe(quote: item.replyQuote()) { quote in
                        model.startReply(.ready(message: quote), in: target)
                    })
                }
                // 点了发送、还没交出去的消息（ADR-0094）跟在最后一条后面；交出去后由时间线里的本地回显代替。
                ForEach(composer.outgoing, id: \.id) { message in
                    OutgoingRow(message: message, composer: composer)
                        .id(message.id)
                        .timelineListRow()
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 1)
            // 键盘收着时也开交互式收起，屏幕靠底一段里行内的按钮点不动，也没有报错（iOS 26.5 模拟器实测，屏高 912：y≈745 能点，y≈805 不能）。
            .scrollDismissesKeyboard(isKeyboardVisible ? .interactively : .never)
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
                isKeyboardVisible = true
            }
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
                isKeyboardVisible = false
            }
            // 翻页把更早的消息插到最上面，List 给的位置不对（iOS 26.5 模拟器实测差了一千多点）：把原来最上面那条留在原位。
            // 第一行变了不一定是翻页（日期分隔线的 id 会重新生成），所以只在翻页途中做。这条离末尾很远，条目变少也不会越界。
            .onChange(of: timeline.items.first?.id) {
                guard let pageAnchor, !pin.isPinned else { return }
                DispatchQueue.main.async { scroller.scrollTo(pageAnchor, anchor: .top) }
            }
            // 待发消息出现、交出去换成本地回显时，贴底的跟着到底；等这一轮排版完再滚。
            .onChange(of: composer.outgoing.map(\.id)) { DispatchQueue.main.async { pin.stick() } }
            .onChange(of: timeline.items.last?.id) { _, last in
                guard last != nil else { return }
                if pin.isPinned, placed { reportPosition(true) }
            }
            .onChange(of: pin.isPinned) { _, pinned in
                // 报告发生在实际滚动之后，不把按钮意图或尚未摆好的首屏当成已读。
                if placed { reportPosition(pinned) }
            }
            // 摆进房间的落点（ADR-0071）：落点不是最新一条就停在它上面（落点行在视野顶部），解开贴底；
            // 没有落点或落点就是最新一条，维持贴底。unpin 触发上面的方向报告，核心的落点由此知道没被当成到底。
            .onChange(of: timeline.isLoaded) { _, loaded in
                if loaded { placeAnchor(scroller) }
            }
            // 时间线清空重建后重摆落点，摆好前一样不标已读。
            .onChange(of: timeline.generation) {
                placed = false
                placeAnchor(scroller)
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { _ in
                pin.stick()
            }
            // 停在别处时的“回到底部”：数字是离开底部后到的消息数（ADR-0071）。
            .overlay(alignment: .bottomTrailing) {
                if case let .away(newMessages) = timeline.viewport {
                    ScrollToBottomButton(count: Int(newMessages)) { pin.pin() }
                }
            }
        }
    }

    /// 摆进房间的落点（ADR-0071）：落点不是最新一条就停在它上面，落点行在视野顶部；没有落点或落点就是最新
    /// 一条，维持贴底。摆好（或决定贴底）后放开标已读的闸门。
    private func placeAnchor(_ scroller: ScrollViewProxy) {
        guard timeline.isLoaded, !placed else { return }
        if let anchor = timeline.anchor, timeline.items.last?.id != anchor,
           timeline.items.contains(where: { $0.id == anchor }) {
            pin.unpin()
            DispatchQueue.main.async {
                scroller.scrollTo(anchor, anchor: .top)
                placed = true
                reportPosition(false)
            }
        } else {
            pin.pin()
            placed = true
            reportPosition(true)
        }
    }

    private func reportPosition(_ pinned: Bool) {
        sendTimeline(.setTimelineScroll(target: target, scroll: pinned ? .bottom : .away))
        // 旁聊页盖着这条主对话时不标已读（ADR-0093）：主人看不见，别替他把新消息读掉。
        if pinned, !covered { sendTimeline(.markRead(target: target)) }
    }
}

@MainActor
private extension ChatRoomView {
    func sendTimeline(_ action: TimelineAction) { model.send(.timeline(action: action)) }

    var isDirect: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isDirect ?? false
    }

    /// 私聊对方是已装应用的 Bot 时非空。
    var bot: BotView? {
        guard let directId = model.viewModel.rooms.first(where: { $0.roomId == roomId })?.directUserId else {
            return nil
        }
        return model.viewModel.bots.first { $0.userId == directId }
    }

    var barAvatar: AnyView? {
        guard let bot else { return nil }
        let plugin = plugins.viewModel.installed?.plugins.first { $0.botUserId == bot.userId }
        return AnyView(BotAvatarView(
            bot: bot, pluginIcon: plugin?.icon, core: model.core, model: plugins, size: WontonTheme.size.avatar
        ).accessibilityIdentifier("roomTopBarAvatar"))
    }

    /// 可以转发到的房间：已加入、能写。
    var forwardRooms: [RoomRow] {
        model.viewModel.rooms.filter { $0.membership == .joined && !$0.isReadOnly }
    }

    var isReadOnly: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isReadOnly ?? false
    }
}

private extension TimelineItem {
    /// 同一个人连着发的消息并成一组：气泡挨紧，相接的角收小。
    func joins(_ other: TimelineItem) -> Bool {
        guard case let .message(mine) = content, case let .message(theirs) = other.content else { return false }
        return mine.senderId == theirs.senderId
    }

    /// 这条消息带的回复引用：核心把引用作为消息动作下发。
    func replyQuote() -> QuotedMessage? {
        guard case let .message(message) = content else { return nil }
        for action in message.actions {
            if case let .reply(quote) = action { return quote }
        }
        return nil
    }

    /// 这条消息带着的确认卡片 ID：Agent 的等待行靠它滚到卡片。
    var cardId: String? {
        guard case let .message(message) = content, case let .confirmation(view) = message.body else { return nil }
        return view.card.id
    }
}

/// 有消息解不开时的一行说明和入口：账号设过备份就输恢复密钥，没设过就请另一台设备确认。都去备份页。
private struct KeyNoticeBanner: View {
    let notice: KeyNotice

    var body: some View {
        WontonNotice(icon: .lockKey, title: String(localized: .Message2.messageUndecryptableNotice), isError: false) {
            // 更新的核心给出的类别没有按钮，说明照常显示。
            if let action = notice.action {
                NavigationLink(value: ChatRoute.chatBackup) {
                    Text(action).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
    }
}

/// 停在别处时的“回到底部”（ADR-0071）：圆形材质按钮，数字在箭头上方的胶囊里，没有数字就不画胶囊。
private struct ScrollToBottomButton: View {
    let count: Int
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 4) {
                if count > 0 {
                    Text(count > Self.badgeMax ? "\(Self.badgeMax)+" : String(count))
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(.thinMaterial, in: Capsule())
                }
                Image(systemName: "arrow.down")
                    .frame(width: 44, height: 44)
                    .background(.regularMaterial, in: Circle())
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(
            count > 0
                ? String(format: String(localized: "room.scrollToBottom.count"), "\(count)")
                : String(localized: "room.scrollToBottom")
        )
        .padding(.trailing, 16)
        .padding(.bottom, 16)
    }

    /// 数字超过这个数就写“99+”。
    private static let badgeMax = 99
}

private extension View {
    func timelineListRow(joinsPrevious: Bool = false, joinsNext: Bool = false) -> some View {
        // 默认样式的按钮放在 List 的行里，点这一行任何地方都会触发。
        buttonStyle(.borderless)
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets(
                top: joinsPrevious ? 1 : WontonTheme.spacing.s4,
                leading: WontonTheme.spacing.pageInset,
                bottom: joinsNext ? 1 : WontonTheme.spacing.s4,
                trailing: WontonTheme.spacing.pageInset
            ))
    }
}

/// 翻页的时机：时间线就绪、条目数或翻页状态变了，再看一次要不要加载更早的消息。
private struct PageRequest: Equatable {
    let isLoaded: Bool
    let count: Int
    let state: BackPagination

    @MainActor
    init(_ timeline: TimelineModel) {
        isLoaded = timeline.isLoaded
        count = timeline.items.count
        state = timeline.backPagination
    }
}

// 这两项只读状态、不碰布局，放在类型体外，免得 `ChatRoomView` 的身体越过 SwiftLint 的 250 行。
private extension ChatRoomView {
    /// 撤回别人的消息时标题写明是谁的。
    var redactTitle: String {
        let target = timeline.items.first { $0.id == redactItem }
        guard case let .message(message)? = target?.content, !message.isOwn else {
            return String(localized: .Message1.messageRedactTitle)
        }
        return String(localized: .Message2.messageRedactOthersTitle(message.senderName))
    }

    /// 分隔线这类虚拟条目的 id 不稳定，定位用消息。
    var oldestEvent: String? {
        timeline.items.first { item in
            switch item.content {
            case .dateDivider, .readMarker, .timelineStart: false
            default: true
            }
        }?.id
    }
}

/// 主对话和旁聊页的顶栏（ADR-0093 §6）：主对话带 Bot 头像和开旁聊的按钮；旁聊页的标题是旁聊的标题，还没
/// 到就先叫“旁聊”，不写状态行、没有头像和按钮。
private struct RoomTopBar<Trailing: View>: ViewModifier {
    let title: String
    let avatar: AnyView?
    /// 旁聊页的标题；主对话为空。
    let sideChatTitle: String?
    /// 旁聊页返回键的动作：收起旁聊，不动导航栈。
    let close: (() -> Void)?
    @ViewBuilder let trailing: () -> Trailing

    func body(content: Content) -> some View {
        if let sideChatTitle {
            content.wontonPushedScreen(sideChatTitle, showsStatus: true, onClose: close) { EmptyView() }
        } else {
            content.wontonPushedScreen(
                title, avatar: avatar, showsStatus: true, showsStatusLine: true, trailing: trailing
            )
        }
    }
}
