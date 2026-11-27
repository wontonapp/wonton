import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let plugins: PluginsModel
    let roomId: String
    /// 旁聊根的事件 ID；空是房间的主时间线（ADR-0093）。旁聊页盖在主对话上（全屏盖层），主对话继续订阅。
    let threadRoot: String?
    let title: String
    @State var timeline: TimelineModel
    @State private var composer: ComposerModel
    @State private var presenter: RoomPresenter
    @State var pin = BottomPin()
    @State private var isKeyboardVisible = false
    @State private var overlayBottom: CGFloat = 0
    @State private var composerHeight: CGFloat = 0
    @State private var pageAnchor: String?
    @SceneStorage private var savedAnchor: String
    @State private var visibleRows: [String: CGFloat] = [:]
    /// 正在选转发目标的条目、正在确认撤回的条目。
    @State private var forwarding: String?
    @State private var redactItem: String?
    @State private var showingRedact = false
    @State private var showingConfirmations = false
    @State private var selectedConfirmation: String?
    @State private var showsBotProfile = false
    @State private var profileBot: BotRef?
    /// 进房间的落点摆好之前不标已读（ADR-0071）：标早了已读行会跳到最新一条，落点就丢了。
    @State private var placed = false
    /// 跳到原消息（ADR-0109 第 4 节）：点的是哪一条的引用、刚跳到要闪一下的那一行、找不到时的轻提示。
    @State var jump = TimelineJumpState()
    /// 输入框上的命令列表开着（ADR-0109 第 7 节）：右下角的“向下”让开，不和它抢地方。
    @State private var commandMenuOpen = false
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
                roomContent
                #if DEBUG
                    .overlay(alignment: .topLeading) {
                        ScrollDebug(
                            pin: pin, keyboard: isKeyboardVisible,
                            overlayBottom: overlayBottom, composerHeight: composerHeight,
                            viewport: String(describing: timeline.viewport),
                            extra: "sub=\(timeline.debugSubscribes) up=\(timeline.debugUpdates)"
                                + " n=\(timeline.items.count)"
                                + " end=\(timeline.debugEnded) closed=\(model.debugClosed)",
                            flashItem: jump.flash?.itemId,
                            flashes: jump.flashes,
                            lastFlash: jump.lastFlash
                        )
                    }
                #endif
            }
        } else {
            roomContent
        }
    }
}

@MainActor
private extension ChatRoomView {
    /// 旁聊页顶栏标题。根还解不开时标题可能是空串，回落本地化「旁聊」。
    private var sideChatBarTitle: String {
        let title = timeline.sideChat?.title ?? ""
        return title.isEmpty ? String(localized: .Chat.sideChat) : title
    }
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
                // 从上到下照 Telegram：插件横幅这类提醒在最上，旁聊标题或提示按在它下面，AI 那一行离内容最近。
                if let plugin = model.viewModel.rooms.first(where: { $0.roomId == roomId })?.plugin {
                    AgentPluginStatusBanner(plugin: plugin, model: plugins)
                }
                if threadRoot != nil {
                    if let state = timeline.sideChat?.state { SideChatHeaderLine(state: state) }
                } else if model.noSideChatsRoom == roomId {
                    NoSideChatsHint()
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
                // 同顶栏：底栏和列表是同层兄弟，输入框会落在列表后面点不到，显式压上去。
                .zIndex(1)
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
        .forwardSheet(item: $forwarding, model: model, plugins: plugins, from: target)
        .navigationDestination(isPresented: $showsBotProfile) {
            if let profileBot {
                BotProfileView(chat: model, plugins: plugins, botRef: profileBot)
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
        // 开新旁聊的面板（ADR-0093 §6）。开成了以后旁聊页由面板的 `afterDismiss` 等面板收完再盖（见 SideChatCover），
        // 两次系统呈现不撞在同一个时机上。
        .modifier(NewSideChatSheet(model: model, roomId: roomId))
        // 盖层见 body 里的 SideChatCover（一张系统全屏页）；“开新旁聊”面板收起和它抬起来会落在同一个呈现时机上。
        .modifier(RoomTopBar(
            title: bot?.name ?? title,
            avatar: barAvatar,
            typing: typingText,
            openProfile: profileAction,
            sideChatTitle: threadRoot == nil ? nil : sideChatBarTitle,
            close: threadRoot == nil ? nil : { model.closeSideChat(in: roomId, why: "bar") }
        ) {
            // 面板跟着主对话（ADR-0093）：旁聊页没有房间的面板，也没有这里的按钮。
            // 房间里装着的 widget（通话）：点了全屏打开，关掉回到这里（ADR-0096）。旁聊页没有。
            if threadRoot == nil, !timeline.widgets.isEmpty {
                RoomCallEntry(widgets: timeline.widgets, active: timeline.callActive) { pages.open($0.tab) }
                    .frame(minWidth: WontonTheme.size.chatControl)
                    .frame(height: WontonTheme.size.chatControl)
                    .chatGlass(in: Capsule(), interactive: true)
            }
            // 和 Bot 的私聊：点顶栏的头像进它的资料页（ADR-0044、ADR-0109 第 9 节），不另放按钮。
            if supportsSideChats {
                ChatGlassButton(
                    icon: .compose, label: String(localized: .Chat.sideChatNew), identifier: "sideChatNew"
                ) {
                    model.composingSideChat = roomId
                }
            } else {
                EmptyView()
            }
        })
        // 入口按钮压进这一页。房间也能从别的标签页打开，那边的导航栈不认聊天页的路由，所以在这里登记一次。
        .modifier(ChatRouteDestinations(model: model, plugins: plugins, isContentRoot: isContentRoot))
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .task(id: model.draftKey(target)) { await model.loadDraft(target) }
        .task(id: model.draftKey(target)) { await composer.observe() }
        .task(id: timeline.attempt) {
            walkLog.notice("task start attempt=\(timeline.attempt, privacy: .public)")
            let ended = await timeline.observe(pin: pin)
            if ended {
                if threadRoot != nil {
                    // 旁聊没了：只收起旁聊，主对话还在。
                    model.closeSideChat(in: roomId, why: "ended")
                } else if !isContentRoot {
                    model.path = []
                    model.visiting = nil
                    pages.open(.chat)
                }
            }
        }
        .onDisappear {
            walkLog.notice("room disappear")
            if threadRoot == nil {
                // 房间关了，开着的旁聊跟着关（换房间、路径被换掉时没有别的收口）。
                model.sideChats[roomId] = nil
                if model.noSideChatsRoom == roomId {
                    model.noSideChatsRoom = nil
                }
            } else if model.sideChats[roomId] == threadRoot {
                // 旁聊页收起时同步关掉：改回主对话就不再算“旁聊开着”。
                model.closeSideChat(in: roomId, why: "gone")
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
                if let notice = botAccessNotice {
                    BotChatNoticeRow(text: notice)
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
                    commandAvatar: { commandAvatar($0) },
                    jumpToReply: { replyJump($0) },
                    menuOpen: commandMenuOpen,
                    openChanged: { commandMenuOpen = $0 },
                    editing: timeline.editing,
                    isEnabled: canSend,
                    disabledProblem: sendBlockedProblem
                )
                .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minY } action: { top in
                    pin.noteComposer(top: top)
                }
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { composerHeight = $0 }
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
                let groups = imageGroups
                ForEach(Array(timeline.items.enumerated()), id: \.element.id) { index, item in
                    // 做完了又没什么可写的 Agent 状态行不占一行。
                    if !item.isBlankAgentRun { row(item, at: index, groups: groups) }
                }
                // 点了发送、还没交出去的消息（ADR-0094）跟在最后一条后面；交出去后由时间线里的本地回显代替。
                ForEach(composer.outgoing, id: \.id) { message in
                    OutgoingRow(message: message, composer: composer)
                        .id(message.id)
                        .timelineListRow()
                }
            }
            // 时间线变了（删了一张、发出去了）：查看器里那组跟着重取，别停在已经不存在的照片上。
            .onChange(of: timeline.items.map(\.id)) { _, _ in presenter.refreshOriginal(imageGroups) }
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
            // 列表按整屏排、把键盘算进 inset，叠层却按整屏底边对齐：按钮要抬过 inset 才不被输入栏盖住。
            .overlay(alignment: .bottomTrailing) {
                GeometryReader { _ in
                    ZStack(alignment: .bottomTrailing) {
                        // 只在 debug 里记 ov=，不放行命中。
                        Color.clear
                            .allowsHitTesting(false)
                            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).maxY } action: {
                                overlayBottom = $0
                            }
                        if case let .away(newMessages) = timeline.viewport, !commandMenuOpen {
                            let lift = isKeyboardVisible ? composerHeight + WontonTheme.spacing.s8 : 0
                            ScrollToBottomButton(count: Int(newMessages), lift: lift) {
                                if timeline.backTo != nil {
                                    sendTimeline(.jumpBack(target: target))
                                } else {
                                    pin.scrollToBottom()
                                    // 核心可能已经算成离开了底部，而壳这边早就被追消息的更新重新贴住：
                                    // 只靠 isPinned 的变化就报不上去，按钮会一直挂着。点它就是到了底。
                                    reportPosition(true)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 这一条属于哪一组的多张照片；不属于时 nil。
    private func photoGroup(_ content: TimelineContent, _ groups: [PhotoGroupKey: [ImageContent]]) -> PhotoGroup? {
        guard case let .message(message) = content, let group = message.group else { return nil }
        let key = PhotoGroupKey(senderId: message.senderId, id: group.id)
        guard let images = groups[key] else { return nil }
        return PhotoGroup(senderId: message.senderId, id: group.id, images: images)
    }

    /// 一次发送里的照片按组收好（只有一张的组也收），给查看器翻页和删除后重取用。
    private var imageGroups: [PhotoGroupKey: [ImageContent]] {
        var byGroup: [PhotoGroupKey: [(index: UInt32, image: ImageContent)]] = [:]
        for item in timeline.items {
            guard case let .message(message) = item.content,
                  let group = message.group,
                  case let .image(image) = message.body else { continue }
            let key = PhotoGroupKey(senderId: message.senderId, id: group.id)
            byGroup[key, default: []].append((group.index, image))
        }
        return byGroup.mapValues { members in
            members.sorted { $0.index < $1.index }.map(\.image)
        }
    }

    private func row(_ item: TimelineItem, at index: Int, groups: [PhotoGroupKey: [ImageContent]]) -> some View {
        let flushesPrevious = index > 0 && timeline.items[index - 1].flushesWith(next: item)
        let flushesNext = index + 1 < timeline.items.count && item.flushesWith(next: timeline.items[index + 1])
        return TimelineRow(
            item: item, core: model.core, showsSender: !isDirect,
            flushesPrevious: flushesPrevious, flushesNext: flushesNext,
            retry: { sendTimeline(.retrySend(target: target, itemId: item.id)) },
            cancel: { sendTimeline(.cancelSend(target: target, itemId: item.id)) },
            cardAction: { model.send($0) },
            reply: { model.startReply(.ready(message: $0), in: target) },
            edit: { text in model.startEdit(text, itemId: item.id, in: target) },
            forward: { forwarding = item.id },
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
            photoGroup: photoGroup(item.content, groups),
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

@MainActor
private extension ChatRoomView {
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

    /// 点输入框上方的引用：和点气泡里的引用一样交给核心跳（ADR-0109 第 4 节）。返回栈记在主人眼下最靠下的一条，
    /// “向下”先回这里。
    func replyJump(_ eventId: String) {
        guard let from = visibleRows.max(by: { $0.value < $1.value })?.key ?? timeline.items.last?.id else { return }
        jump.from = from
        sendTimeline(.jumpToMessage(target: target, eventId: eventId, fromItemId: from))
    }

    /// 摆进房间的落点（ADR-0071）：落点不是最新一条就停在它上面，落点行在视野顶部；没有落点或落点就是最新
    /// 一条，维持贴底。摆好（或决定贴底）后放开标已读的闸门。
    private func placeAnchor(_ scroller: ScrollViewProxy) {
        guard timeline.isLoaded, !placed else { return }
        if !savedAnchor.isEmpty {
            pin.unpin()
            guard timeline.items.contains(where: { $0.id == savedAnchor }) else { return }
            let anchor = savedAnchor
            settle(anchor, at: .top, scroller, corrections: Self.placeCorrections) {}
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
            settle(anchor, at: .top, scroller, corrections: Self.placeCorrections) {}
            DispatchQueue.main.async {
                placed = true
                reportPosition(false)
            }
        } else {
            pin.scrollToBottom()
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

    /// 点顶栏的头像：和 Bot 的私聊进它的资料页（ADR-0044），别的房间没有资料页。
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

    /// 顶栏状态行的“正在输入…”（ADR-0109 第 8 节）：私聊不写名字，群里写第一个人，多人时加“和另外 N 人”。
    /// 谁在输入、什么时候算停都由核心给。
    var typingText: String? {
        guard let first = timeline.typing.first else { return nil }
        if isDirect { return String(localized: .Chat.roomTyping) }
        let others = timeline.typing.count - 1
        return others == 0
            ? String(localized: .Chat.roomTypingOne(first))
            : String(localized: .Chat.roomTypingMany(first, others))
    }

    var isReadOnly: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isReadOnly ?? false
    }

    /// 这个私聊的 Bot 现在能不能聊；房间里没这条事实时退回 Bot 列表，普通人私聊为空。
    var botAccess: BotAccess? {
        let row = model.viewModel.rooms.first { $0.roomId == roomId }
        if let access = row?.bot?.access { return access }
        return row?.directUserId.flatMap { user in
            model.viewModel.bots.first { $0.userId == user }.map { asBotAccess($0.availability) }
        }
    }

    /// 不能聊时输入框上方写一句为什么；能用或不是 Bot 时为空。
    var botAccessNotice: LocalizedStringResource? {
        guard let access = botAccess else { return nil }
        return botAccessNoticeText(access)
    }

    /// 确定的不能用才拦发送；资格未知（离线、老核心）照常发，进离线队列。
    var canSend: Bool {
        guard let access = botAccess else { return true }
        if case .unusable = access { return false }
        return true
    }

    var sendBlockedProblem: Problem? {
        guard !canSend, let notice = botAccessNotice else { return nil }
        return Problem(title: String(localized: notice))
    }
}

/// 不能聊的原因；能用时为空。新建聊天和房间提示共用同一套文字。
func botAccessNoticeText(_ access: BotAccess) -> LocalizedStringResource? {
    switch access {
    case .own, .shared, .contact:
        return nil
    case .unknown:
        return .Chat.roomBotUnknown
    case let .unusable(reason):
        switch reason {
        case .ownerPrivate: return .Chat.roomBotOwnerPrivate
        case .grantNotActive: return .Chat.roomBotGrantRevoked
        case .roleLacksSharedBots: return .Chat.roomBotRoleLacksSharedBots
        case .appNotShared: return .Chat.roomBotAppNotShared
        case .contactPending: return .Chat.roomBotContactPending
        case .contactDeclined: return .Chat.roomBotContactDeclined
        case .botMissing: return .Chat.roomBotMissing
        default: return .Chat.roomBotUnavailable
        }
    }
}

/// 旧字段只有能不能时，补一个按原因分类的访问结论，供只认 BotAccess 的界面用。
private func asBotAccess(_ availability: BotChatAvailability) -> BotAccess {
    switch availability {
    case .available: return .contact
    case .unknown: return .unknown
    case let .unavailable(reason): return .unusable(reason: reason)
    }
}

/// 不能聊时输入框上方的一句话；房间和历史照留，只是不发。
private struct BotChatNoticeRow: View {
    let text: LocalizedStringResource
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(text)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textSecondary))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s4)
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

    /// 气泡那一行贴近屏幕边（ADR-0109 第 9 节），其余的行照页面的边距。
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
        guard
            case let .agentRun(_, state, _, steps, _, result, _, _, _, choices, _, _, _) = content
        else { return false }
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

/// 聊天页登记导航路由。旁聊页有自己的栈（盖层里），各登记一次互不影响。
struct ChatRouteDestinations: ViewModifier {
    let model: ChatModel
    let plugins: PluginsModel
    let isContentRoot: Bool

    func body(content: Content) -> some View {
        content.navigationDestination(for: ChatRoute.self) { route in
            if isContentRoot {
                ChatDestination(model: model, plugins: plugins, route: route)
                    .environment(\.wontonContentRoot, false)
            } else if case .chatBackup = route {
                ChatBackupView(model: model)
                    .environment(\.wontonContentRoot, false)
            }
        }
    }
}
