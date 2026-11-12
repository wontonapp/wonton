import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let plugins: PluginsModel
    let roomId: String
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
    @Environment(TabsModel.self) private var tabs

    init(model: ChatModel, plugins: PluginsModel, roomId: String, title: String) {
        self.model = model
        self.plugins = plugins
        self.roomId = roomId
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId))
        _composer = State(initialValue: ComposerModel(chat: model, roomId: roomId))
        _presenter = State(initialValue: RoomPresenter(files: model.files))
    }

    var body: some View {
        RoomPanelHost(roomId: roomId, panel: timeline.panel, selected: timeline.panelSelected,
                      blocked: entryBlocked, pullAnchor: roomPull) {
            roomContent
                .overlay(alignment: .topLeading) { ScrollDebug(text: pin.snapshot) }
        }
    }

    /// 撤回别人的消息时标题写明是谁的。
    private var redactTitle: String {
        let target = timeline.items.first { $0.id == redactItem }
        guard case let .message(message)? = target?.content, !message.isOwn else {
            return String(localized: .messageRedactTitle)
        }
        return String(localized: .messageRedactOthersTitle(message.senderName))
    }

    /// 房间里另有东西开着：长按菜单之外的转发、删除确认不响应上推入口（ADR-0052）。
    private var entryBlocked: Bool { pickingForwardRoom || showingRedact }

    private var roomContent: some View {
        VStack(spacing: 0) {
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
                if let bot = onboarding.viewModel.primaryBotUserId,
                   model.viewModel.rooms.first(where: { $0.roomId == roomId })?.directUserId == bot,
                   !onboarding.viewModel.chatPrompts.isEmpty {
                    OnboardingChatPrompts(prompts: onboarding.viewModel.chatPrompts) {
                        model.appendDraft($0, to: roomId)
                    }
                }
                ComposerView(model: model, composer: composer, pin: pin)
            }
        }
        .modifier(RoomPresentations(presenter: presenter))
        .wontonSheet(isPresented: $pickingForwardRoom, title: String(localized: .messageForwardTitle)) { close in
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
            message: String(localized: .messageRedactMessage),
            isPresented: $showingRedact,
            actions: [
                WontonDialogAction(title: String(localized: .commonCancel)),
                WontonDialogAction(title: String(localized: .messageRedact), kind: .danger) {
                    if let item = redactItem { sendTimeline(.redactMessage(target: mainTimeline, itemId: item)) }
                }
            ],
            afterDismiss: { redactItem = nil }
        )
        .environment(model.files)
        .environment(presenter)
        // 和 Bot 的私聊：顶栏是它的头像和 Matrix 名字，不写状态文字（ADR-0044）。
        .wontonPushedScreen(
            bot?.name ?? title, avatar: barAvatar, showsStatus: true, showsStatusLine: true
        ) { EmptyView() }
        // 入口按钮压进这一页。房间也能从别的标签页打开，那边的导航栈不认聊天页的路由，所以在这里登记一次。
        .navigationDestination(for: ChatRoute.self) { route in
            if case .chatBackup = route { ChatBackupView(model: model) }
        }
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .onChange(of: entryBlocked, initial: true) { _, blocked in
            model.send(.setRoomEntryBlocked(roomId: roomId, blocked: blocked))
        }
        .task(id: roomId) { await model.loadDraft(roomId) }
        .task(id: roomId) { await composer.observe() }
        .task(id: timeline.attempt) {
            if await timeline.observe(pin: pin) {
                model.path = []
                model.visiting = nil
                tabs.select(.chat)
            }
        }
    }

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
                                    sendTimeline(.paginateBackwards(target: mainTimeline))
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
                        retry: { sendTimeline(.retrySend(target: mainTimeline, itemId: item.id)) },
                        cancel: { sendTimeline(.cancelSend(target: mainTimeline, itemId: item.id)) },
                        cardAction: { model.send($0) },
                        reply: { model.startReply(.ready(message: $0), in: roomId) },
                        forward: {
                            forwarding = item.id
                            pickingForwardRoom = true
                        },
                        redact: {
                            redactItem = item.id
                            showingRedact = true
                        }
                    )
                    .id(item.id)
                    .timelineListRow(joinsPrevious: joinsPrevious, joinsNext: joinsNext)
                    .modifier(ReplySwipe(quote: item.replyQuote()) { quote in
                        model.startReply(.ready(message: quote), in: roomId)
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

    /// 分隔线这类虚拟条目的 id 不稳定，定位用消息。
    private var oldestEvent: String? {
        timeline.items.first { item in
            switch item.content {
            case .dateDivider, .readMarker, .timelineStart: false
            default: true
            }
        }?.id
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
        sendTimeline(.setTimelineScroll(target: mainTimeline, scroll: pinned ? .bottom : .away))
        if pinned { sendTimeline(.markRead(target: mainTimeline)) }
    }

    private var isDirect: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isDirect ?? false
    }

    /// 私聊对方是已装应用的 Bot 时非空。
    private var bot: BotView? {
        guard let directId = model.viewModel.rooms.first(where: { $0.roomId == roomId })?.directUserId else {
            return nil
        }
        return model.viewModel.bots.first { $0.userId == directId }
    }

    private var barAvatar: AnyView? {
        guard let bot else { return nil }
        let plugin = plugins.viewModel.installed?.plugins.first { $0.botUserId == bot.userId }
        return AnyView(BotAvatarView(
            bot: bot, pluginIcon: plugin?.icon, core: model.core, model: plugins, size: WontonTheme.size.avatar
        ).accessibilityIdentifier("roomTopBarAvatar"))
    }

    /// 可以转发到的房间：已加入、能写。
    private var forwardRooms: [RoomRow] {
        model.viewModel.rooms.filter { $0.membership == .joined && !$0.isReadOnly }
    }

    private var isReadOnly: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isReadOnly ?? false
    }
}

@MainActor
private extension ChatRoomView {
    var mainTimeline: TimelineTarget { TimelineTarget(roomId: roomId, threadRoot: nil) }

    func sendTimeline(_ action: TimelineAction) { model.send(.timeline(action: action)) }
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
}

/// 有消息解不开时的一行说明和入口：账号设过备份就输恢复密钥，没设过就请另一台设备确认。都去备份页。
private struct KeyNoticeBanner: View {
    let notice: KeyNotice

    var body: some View {
        WontonNotice(icon: .lockKey, title: String(localized: .messageUndecryptableNotice), isError: false) {
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
