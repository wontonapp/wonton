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
    @SceneStorage private var savedAnchor: String
    @State private var visibleRows: [String: CGFloat] = [:]
    /// 正在选转发目标的条目、正在确认撤回的条目。
    @State private var forwarding: String?
    @State private var pickingForwardRoom = false
    @State private var redactItem: String?
    @State private var showingRedact = false
    @State private var showingConfirmations = false
    @State private var selectedConfirmation: String?
    @State private var showsBotProfile = false
    @State private var profileBot: BotRef?
    /// 进房间的落点摆好之前不标已读（ADR-0071）：标早了已读行会跳到最新一条，落点就丢了。
    @State private var placed = false
    /// 跳到原消息（ADR-0109 第 4 节）：点的是哪一条的引用、刚跳到要闪一下的那一行、找不到时的轻提示。
    @State private var jump = TimelineJumpState()
    @Environment(OnboardingModel.self) private var onboarding
    @Environment(PagesModel.self) private var pages
    @Environment(\.wontonContentRoot) private var isContentRoot

    init(model: ChatModel, plugins: PluginsModel, roomId: String, threadRoot: String? = nil, title: String) {
        self.model = model
        self.plugins = plugins
        self.roomId = roomId
        self.threadRoot = threadRoot
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId, threadRoot: threadRoot))
        _composer = State(initialValue: ComposerModel(chat: model, roomId: roomId, threadRoot: threadRoot))
        _presenter = State(initialValue: RoomPresenter(files: model.files, roomId: roomId))
        let key = model.draftKey(TimelineTarget(roomId: roomId, threadRoot: threadRoot))
        _savedAnchor = SceneStorage(wrappedValue: "", "timeline.anchor.\(key)")
    }

    var body: some View {
        if threadRoot == nil {
            // 旁聊页全屏盖在这条主对话上（ADR-0093）：盖层的机制在 SideChatCover。
            SideChatCover(model: model, plugins: plugins, roomId: roomId, title: title) {
                RoomPanelHost(roomId: roomId, panel: timeline.panel, selected: timeline.panelSelected) {
                    roomContent
                    #if DEBUG
                        .overlay(alignment: .topLeading) { ScrollDebug(pin: pin) }
                    #endif
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
    private var roomContent: some View {
        Group {
            if let failure = timeline.failure {
                ProblemView(problem: failure) { timeline.attempt += 1 }
                    .frame(maxHeight: .infinity)
            } else {
                messages
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            // 顶栏下面这几行垫材质：列表从它们底下滚过去，不垫的话气泡里的字和它们叠在一起。
            VStack(spacing: 0) {
                if threadRoot != nil {
                    if let state = timeline.sideChat?.state { SideChatHeaderLine(state: state) }
                } else if model.noSideChatsRoom == roomId {
                    NoSideChatsHint()
                }
                if let plugin = model.viewModel.rooms.first(where: { $0.roomId == roomId })?.plugin {
                    AgentPluginStatusBanner(plugin: plugin, model: plugins)
                }
                // 群里能用的 AI；私聊不显示，Bot 本来就在顶栏（ADR-0044）。
                if threadRoot == nil, !isDirect {
                    RoomAiLine(
                        roomBots: timeline.roomBots,
                        bots: model.viewModel.bots,
                        core: model.core,
                        plugins: plugins
                    ) { openProfile($0) }
                }
            }
            .frame(maxWidth: .infinity)
            .floatingChatCard()
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            roomBottomBar
        }
        .modifier(RoomPresentations(presenter: presenter))
        .wontonSheet(isPresented: $showingConfirmations,
            title: String(localized: .ConfirmationCard.confirmationCardRequests)) { _ in
            RoomConfirmationSheet(
                cards: timeline.confirmations, selectedId: selectedConfirmation,
                action: { model.send($0) }
            )
            .environment(pages)
            .environment(plugins)
        }
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
        .navigationDestination(isPresented: $showsBotProfile) {
            if let profileBot { BotProfileView(chat: model, plugins: plugins, botRef: profileBot) }
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
            typing: typingText,
            openProfile: profileAction,
            sideChatTitle: threadRoot == nil ? nil : timeline.sideChat?.title ?? String(localized: .Chat.sideChat),
            close: threadRoot == nil ? nil : { model.closeSideChat(in: roomId) }
        ) {
            // 面板跟着主对话（ADR-0093）：旁聊页没有房间的面板，也没有这里的按钮。
            if threadRoot == nil {
                PanelEntryButton(floats: true)
            }
            // 房间里装着的 widget（通话）：点了全屏打开，关掉回到这里（ADR-0096）。旁聊页没有。
            if threadRoot == nil, !timeline.widgets.isEmpty {
                RoomCallEntry(widgets: timeline.widgets, active: timeline.callActive) { pages.open($0.tab) }
                    .frame(minWidth: WontonTheme.size.chatControl)
                    .frame(height: WontonTheme.size.chatControl)
                    .chatGlass(in: Capsule(), interactive: true)
            }
            // 和 Bot 的私聊：点顶栏的头像或标题进它的资料页（ADR-0044、ADR-0109 第 8 节），不另放按钮。
            if supportsSideChats {
                ChatGlassButton(icon: .compose, label: String(localized: .Chat.sideChatNew)) {
                    model.composingSideChat = roomId
                }
            } else {
                EmptyView()
            }
        })
        // 入口按钮压进这一页。房间也能从别的标签页打开，那边的导航栈不认聊天页的路由，所以在这里登记一次。
        .navigationDestination(for: ChatRoute.self) { route in
            if isContentRoot {
                ChatDestination(model: model, plugins: plugins, route: route)
                    .environment(\.wontonContentRoot, false)
            } else if case .chatBackup = route {
                ChatBackupView(model: model)
                    .environment(\.wontonContentRoot, false)
            }
        }
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .task(id: model.draftKey(target)) { await model.loadDraft(target) }
        .task(id: model.draftKey(target)) { await composer.observe() }
        .task(id: timeline.attempt) {
            if await timeline.observe(pin: pin) {
                if threadRoot != nil {
                    // 旁聊没了：只收起旁聊，主对话还在。
                    model.closeSideChat(in: roomId)
                } else if !isContentRoot {
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

    private var roomBottomBar: some View {
        VStack(spacing: WontonTheme.spacing.s4) {
            // 输入栏上方的几条提示收在一张浮着的玻璃卡片里；一条都没有时卡片没有高度，不画。
            VStack(spacing: 0) {
                if let notice = timeline.notice {
                    KeyNoticeBanner(notice: notice)
                }
                TransfersStrip(roomId: roomId)
                RoomConfirmationBanner(cards: timeline.confirmations) {
                    selectedConfirmation = nil
                    showingConfirmations = true
                }
                if !isReadOnly, threadRoot == nil, onboarding.viewModel.defaultBotVisibility == .visible,
                   let bot = onboarding.viewModel.defaultBotUserId,
                   model.viewModel.rooms.first(where: { $0.roomId == roomId })?.directUserId == bot,
                   !onboarding.viewModel.chatPrompts.isEmpty {
                    OnboardingChatPrompts(prompts: onboarding.viewModel.chatPrompts) {
                        model.appendDraft($0, to: target)
                    }
                }
            }
            .frame(maxWidth: .infinity)
            .floatingChatCard()
            if !isReadOnly {
                ComposerView(
                    model: model, composer: composer, pin: pin,
                    commands: timeline.commands, commandsOmitted: Int(timeline.commandsOmitted),
                    commandAvatar: { commandAvatar($0) }
                )
            }
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
                                if timeline.isLoaded, timeline.backPagination == .idle, placed {
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
                .background(EnclosingScrollView { pin.attach($0) })
                .timelineListRow()
                ForEach(Array(timeline.items.enumerated()), id: \.element.id) { index, item in
                    // 做完了又没什么可写的 Agent 状态行不占一行。
                    if !item.isBlankAgentRun { row(item, at: index) }
                }
                // 点了发送、还没交出去的消息（ADR-0094）跟在最后一条后面；交出去后由时间线里的本地回显代替。
                ForEach(composer.outgoing, id: \.id) { message in
                    OutgoingRow(message: message, composer: composer)
                        .id(message.id)
                        .timelineListRow()
                }
            }
            .listStyle(.plain)
            .coordinateSpace(name: "timelineViewport")
            .task(id: PageRequest(timeline)) {
                guard timeline.isLoaded, !placed, !savedAnchor.isEmpty else { return }
                if timeline.items.contains(where: { $0.id == savedAnchor }) {
                    placeAnchor(scroller)
                } else if timeline.backPagination == .idle {
                    sendTimeline(.paginateBackwards(target: target))
                } else if timeline.backPagination == .reachedStart {
                    savedAnchor = ""
                    placeAnchor(scroller)
                }
            }
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
                if placed, pinned { savedAnchor = "" }
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
            .onGeometryChange(for: EdgeInsets.self) { $0.safeAreaInsets } action: { _ in
                // Bar growth changes scroll insets, not the underlapping list's frame.
                DispatchQueue.main.async { pin.stick() }
            }
            .onChange(of: timeline.focusRevision) { focus(scroller) }
            .onChange(of: timeline.jump) { _, state in
                if state != .loading { jump.from = nil }
                if state == .unavailable { jump.missing += 1 }
            }
            .task(id: jump.missing) {
                guard jump.missing > 0 else { return }
                jump.showsMissing = true
                let hold = WontonTheme.motion.jumpHighlight + WontonTheme.motion.datePillHold
                guard (try? await Task.sleep(for: .seconds(hold))) != nil else { return }
                jump.showsMissing = false
            }
            .overlay(alignment: .top) { FloatingDatePill(timestampMs: topVisibleTimestamp, pin: pin) }
            .overlay(alignment: .bottom) {
                if jump.showsMissing { JumpMissingHint().transition(.opacity) }
            }
            // 停在别处时的“向下”：数字是离开底部后到的消息数（ADR-0071）。跳过原消息的先回到跳之前那一条，
            // 栈空了才到底（ADR-0109 第 4 节）。
            .overlay(alignment: .bottomTrailing) {
                if case let .away(newMessages) = timeline.viewport {
                    ScrollToBottomButton(count: Int(newMessages)) {
                        if timeline.backTo != nil {
                            sendTimeline(.jumpBack(target: target))
                        } else {
                            pin.pin()
                        }
                    }
                }
            }
        }
    }

    private func row(_ item: TimelineItem, at index: Int) -> some View {
        let flushesPrevious = index > 0 && timeline.items[index - 1].flushesWith(next: item)
        let flushesNext = index + 1 < timeline.items.count && item.flushesWith(next: timeline.items[index + 1])
        return TimelineRow(
            item: item, core: model.core, showsSender: !isDirect,
            flushesPrevious: flushesPrevious, flushesNext: flushesNext,
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
                selectedConfirmation = cardId
                showingConfirmations = true
            },
            answerChoice: { key in
                model.send(.chat(action: .answerAgentChoice(roomId: roomId, itemId: item.id, key: key)))
            },
            jump: { eventId in
                jump.from = item.id
                sendTimeline(.jumpToMessage(target: target, eventId: eventId, fromItemId: item.id))
            },
            isJumping: timeline.jump == .loading && jump.from == item.id,
            avatar: isDirect ? nil : { senderAvatar($0) },
            onOpenSideChat: { model.openSideChat($0, in: roomId) }
        )
        .id(item.id)
        .onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .named("timelineViewport"))
        } action: { frame in
            if case .message = item.content, frame.maxY > 0, frame.minY < pin.viewportHeight {
                visibleRows[item.id] = frame.minY
            } else {
                visibleRows.removeValue(forKey: item.id)
            }
            saveVisibleAnchor()
        }
        .onDisappear { visibleRows.removeValue(forKey: item.id) }
        .timelineListRow(
            top: item.gapAbove, bottom: item.gapBelow, side: item.sideInset,
            highlight: jump.flash?.itemId == item.id ? jump.flash?.revision : nil
        )
        .modifier(ReplyDrag(quote: item.replyQuote()) { quote in
            model.startReply(.ready(message: quote), in: target)
        })
    }
}

/// 跳到原消息在壳这边的状态（ADR-0109 第 4 节）；定位、返回栈都在核心。
private struct TimelineJumpState {
    /// 点的是哪一条的引用：核心往前翻页时在它上面画加载。
    var from: String?
    /// 刚跳到、要闪一下的那一行，和这是第几次跳。
    var flash: (itemId: String, revision: Int)?
    /// 原消息找不到的次数：每多一次，轻提示再出一回。
    var missing = 0
    var showsMissing = false
}

@MainActor
private extension ChatRoomView {
    /// 核心说滚到哪条就把哪条摆到可视区中部；跳到原消息时那一行闪一下，闪完把记号清掉（不然这一行滚出去再回来会再闪）。
    func focus(_ scroller: ScrollViewProxy) {
        guard let focus = timeline.focus else { return }
        let revision = timeline.focusRevision
        jump.from = nil
        pin.unpin()
        settle(focus.itemId, at: .center, scroller, corrections: Self.focusCorrections) {
            guard focus.highlight, revision == timeline.focusRevision else { return }
            jump.flash = (focus.itemId, revision)
            let lit = WontonTheme.motion.jumpHighlight + WontonTheme.motion.panel
            DispatchQueue.main.asyncAfter(deadline: .now() + lit) {
                if jump.flash?.revision == revision { jump.flash = nil }
            }
        }
    }

    /// 离得远的行还没量过高度，第一次滚到的位置按估算的行高算、会偏：等这一轮排完再对几次。
    /// 滚之前先看这条还在不在：`scrollTo` 找不到条目会越界崩掉（见 BottomPin）。
    /// 主人自己滚走了（贴底了，或者在拖）就不再对。
    func settle(
        _ itemId: String, at anchor: UnitPoint, _ scroller: ScrollViewProxy, corrections: Int,
        after delay: TimeInterval = 0, done: @escaping () -> Void
    ) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            guard !pin.isPinned, !pin.isTouched,
                  timeline.items.contains(where: { $0.id == itemId }) else { return }
            scroller.scrollTo(itemId, anchor: anchor)
            guard corrections > 0 else { return done() }
            settle(
                itemId, at: anchor, scroller, corrections: corrections - 1, after: WontonTheme.motion.feedback,
                done: done
            )
        }
    }

    /// 滚到一条之后再对几次位置。
    static var focusCorrections: Int { 2 }

    /// 可见区最上面那条消息的发送时间：浮动的日期胶囊写它的日期。
    var topVisibleTimestamp: UInt64? {
        guard let top = visibleRows.min(by: { $0.value < $1.value })?.key,
              case let .message(message)? = timeline.items.first(where: { $0.id == top })?.content else { return nil }
        return message.timestampMs
    }

    /// 群聊里组尾气泡旁的头像（ADR-0109 第 2 节）：核心给的 `senderAvatar`，没有图片时画名字的第一个字；
    /// 图片取到之前先露出底下的字。
    func senderAvatar(_ message: TimelineMessage) -> AnyView {
        let size = WontonTheme.size.messageAvatar
        return AnyView(ZStack {
            SenderInitial(name: message.senderName)
            if let media = message.senderAvatar {
                StillAvatarImage(media: media, core: model.core, size: size)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle()))
    }
}

private extension ChatRoomView {

    /// 摆进房间的落点（ADR-0071）：落点不是最新一条就停在它上面，落点行在视野顶部；没有落点或落点就是最新
    /// 一条，维持贴底。摆好（或决定贴底）后放开标已读的闸门。
    private func placeAnchor(_ scroller: ScrollViewProxy) {
        guard timeline.isLoaded, !placed else { return }
        if !savedAnchor.isEmpty {
            pin.unpin()
            guard timeline.items.contains(where: { $0.id == savedAnchor }) else { return }
            let anchor = savedAnchor
            settle(anchor, at: .top, scroller, corrections: Self.focusCorrections) {}
            DispatchQueue.main.async {
                guard timeline.items.contains(where: { $0.id == anchor }) else { return }
                placed = true
                reportPosition(false)
            }
            return
        }
        if let anchor = timeline.anchor, timeline.items.last?.id != anchor,
           timeline.items.contains(where: { $0.id == anchor }) {
            pin.unpin()
            // 列表还没排完时这一次滚动不生效，停在最上面（iOS 26.5 模拟器 2026-11-19 实测，六次进房间有一次）：再对几次。
            settle(anchor, at: .top, scroller, corrections: Self.focusCorrections) {}
            DispatchQueue.main.async {
                placed = true
                reportPosition(false)
            }
        } else {
            pin.pin()
            placed = true
            reportPosition(true)
        }
    }

    private func saveVisibleAnchor() {
        guard placed, !pin.isPinned else { return }
        if let first = visibleRows.min(by: { $0.value < $1.value }),
           timeline.items.contains(where: { $0.id == first.key }) {
            savedAnchor = first.key
        }
    }

    private func reportPosition(_ pinned: Bool) {
        timeline.setScroll(pinned ? .bottom : .away)
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

    /// 顶栏右边的圆头像：和聊天列表那一行是同一个（Bot 头像、应用图标或名字的第一个字）。
    var barAvatar: AnyView? {
        guard let row = model.viewModel.rooms.first(where: { $0.roomId == roomId }) else { return nil }
        return AnyView(RoomAvatar(row: row, model: model, plugins: plugins, size: WontonTheme.size.chatControl))
    }

    /// 点顶栏的头像或标题：和 Bot 的私聊进它的资料页（ADR-0044），别的房间没有资料页。
    var profileAction: (() -> Void)? {
        guard let bot else { return nil }
        return { openProfile(bot.bot) }
    }

    func openProfile(_ bot: BotRef) {
        profileBot = bot
        showsBotProfile = true
    }

    /// 命令列表里每条命令旁的头像：这条命令是哪个 Bot 的就画谁；不是已装应用的 Bot 画名字的第一个字。
    func commandAvatar(_ command: RoomCommand) -> AnyView {
        let size = WontonTheme.size.commandAvatar
        guard let bot = model.viewModel.bots.first(where: { $0.userId == command.botUserId }) else {
            return AnyView(SenderInitial(name: command.botName, size: size))
        }
        let plugin = plugins.viewModel.installed?.plugins.first { $0.id == bot.bot.pluginId }
        return AnyView(
            BotAvatarView(bot: bot, pluginIcon: plugin?.icon, core: model.core, model: plugins, size: size)
                .clipShape(Circle())
        )
    }

    /// 顶栏状态行的“正在输入…”（ADR-0109 第 7 节）：私聊不写名字，群里写第一个人，多人时加“和另外 N 人”。
    /// 谁在输入、什么时候算停都由核心给。
    var typingText: String? {
        guard let first = timeline.typing.first else { return nil }
        if isDirect { return String(localized: .Chat.roomTyping) }
        let others = timeline.typing.count - 1
        return others == 0
            ? String(localized: .Chat.roomTypingOne(first))
            : String(localized: .Chat.roomTypingMany(first, others))
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
    /// 这一行上下各留多少：同一组里的两条之间收紧，组和组之间留大间距。组内位置由核心给（ADR-0109），
    /// 壳不比发送者和时间。相邻两行各出一半。
    var gapAbove: CGFloat {
        guard case let .message(message) = content, message.run.joinsPrevious else {
            return WontonTheme.spacing.groupGap / 2
        }
        return WontonTheme.spacing.runGap / 2
    }

    var gapBelow: CGFloat {
        guard case let .message(message) = content, message.run.joinsNext else {
            return WontonTheme.spacing.groupGap / 2
        }
        return WontonTheme.spacing.runGap / 2
    }

    /// 气泡那一行贴近屏幕边（ADR-0109 第 8 节），其余的行照页面的边距。
    var sideInset: CGFloat {
        if case .message = content { return WontonTheme.spacing.bubbleEdge }
        return WontonTheme.spacing.pageInset
    }

    /// 这条和紧跟着的 `next` 之间能不能不留衬垫贴成一摞：同一次发送（ADR-0094 的组标记）里挨着的两张照片，
    /// `next` 不带引用、这条不带说明。
    func flushesWith(next: TimelineItem) -> Bool {
        guard case let .message(mine) = content, case let .message(theirs) = next.content,
            let group = mine.group, theirs.group?.id == group.id, theirs.reply == nil
        else { return false }
        guard case let .image(mineImage) = mine.body, case .image = theirs.body else { return false }
        return mineImage.caption == nil
    }

    var isBlankAgentRun: Bool {
        guard case let .agentRun(_, state, _, steps, _, result, _, _, _, choices, _, _) = content else { return false }
        return choices.isEmpty && AgentRunRow.isBlank(state: state, steps: steps, result: result)
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

private extension View {
    func timelineListRow(
        top: CGFloat = WontonTheme.spacing.groupGap / 2, bottom: CGFloat = WontonTheme.spacing.groupGap / 2,
        side: CGFloat = WontonTheme.spacing.pageInset, highlight: Int? = nil
    ) -> some View {
        // 默认样式的按钮放在 List 的行里，点这一行任何地方都会触发。
        buttonStyle(.borderless)
            .listRowSeparator(.hidden)
            .listRowBackground(JumpHighlight(trigger: highlight))
            .listRowInsets(EdgeInsets(
                top: top,
                leading: side,
                bottom: bottom,
                trailing: side
            ))
    }

    /// 顶栏下面、输入栏上面那几条提示的底：一张浮在消息上的玻璃卡片（ADR-0107）。
    func floatingChatCard() -> some View {
        let shape = RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous)
        return clipShape(shape)
            .chatGlass(in: shape)
            .padding(.horizontal, WontonTheme.spacing.chatFloatInset)
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
