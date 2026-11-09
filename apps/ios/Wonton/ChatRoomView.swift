import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let plugins: PluginsModel
    let roomId: String
    let title: String
    @State private var timeline: TimelineModel
    @State private var presenter: RoomPresenter
    @State private var pin = BottomPin()
    @State private var isKeyboardVisible = false
    @State private var pageAnchor: String?
    @Environment(OnboardingModel.self) private var onboarding
    @Environment(TabsModel.self) private var tabs

    init(model: ChatModel, plugins: PluginsModel, roomId: String, title: String) {
        self.model = model
        self.plugins = plugins
        self.roomId = roomId
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId))
        _presenter = State(initialValue: RoomPresenter(files: model.files))
    }

    var body: some View {
        VStack(spacing: 0) {
            if let failure = timeline.failure {
                ProblemView(problem: failure) { timeline.attempt += 1 }
                    .frame(maxHeight: .infinity)
            } else {
                messages
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
                ComposerView(model: model, roomId: roomId, pin: pin)
            }
        }
        .modifier(RoomPresentations(presenter: presenter))
        .environment(model.files)
        .environment(presenter)
        // 和 Bot 的私聊：顶栏是它的头像和 Matrix 名字，不写状态文字（ADR-0044）。
        .wontonPushedScreen(bot?.name ?? title, avatar: barAvatar, showsStatus: true) { EmptyView() }
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .task(id: roomId) { await model.loadDraft(roomId) }
        .task(id: timeline.attempt) {
            if await timeline.observe() {
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
                                    model.send(.paginateBackwards(roomId: roomId))
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
                    let joinsPrevious = index > 0 && item.joins(timeline.items[index - 1])
                    let joinsNext = index + 1 < timeline.items.count && item.joins(timeline.items[index + 1])
                    TimelineRow(
                        item: item, core: model.core, showsSender: !isDirect,
                        joinsPrevious: joinsPrevious, joinsNext: joinsNext,
                        retry: { model.send(.retrySend(roomId: roomId, itemId: item.id)) },
                        cancel: { model.send(.cancelSend(roomId: roomId, itemId: item.id)) },
                        cardAction: { model.send($0) }
                    )
                    .id(item.id)
                    .timelineListRow(joinsPrevious: joinsPrevious, joinsNext: joinsNext)
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
            .onChange(of: timeline.items.last?.id) { _, last in
                if last != nil { model.send(.markRoomRead(roomId: roomId)) }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { _ in
                pin.stick()
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

    private var isReadOnly: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isReadOnly ?? false
    }
}

private extension TimelineItem {
    /// 同一个人连着发的消息并成一组：气泡挨紧，相接的角收小。
    func joins(_ other: TimelineItem) -> Bool {
        guard case let .message(mine) = content, case let .message(theirs) = other.content else { return false }
        return mine.senderId == theirs.senderId
    }
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
