import Observation
import WontonClient

@MainActor
@Observable
final class TimelineModel {
    private(set) var items: [TimelineItem] = []
    private(set) var confirmations: [ConfirmationView] = []
    private(set) var backPagination: BackPagination = .idle
    /// 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。
    private(set) var isLoaded = false
    /// 核心算的位置：在不在底部、离开底部时的新消息数（ADR-0071）。
    private(set) var viewport: TimelineViewport = .bottom
    /// 第一批的落点：进房间停在这一条上（ADR-0071）。
    private(set) var anchor: String?
    /// 有消息解不开时给主人的入口，核心按加密状态给。
    private(set) var notice: KeyNotice?
    /// 时间线被清空重建的次数：重建的 Reset 里带新落点，视图要重新摆。
    private(set) var generation = 0
    /// 走查用（DEBUG）：订阅任务进来了几次、收到了几批，挂在 `ScrollDebug` 的自述里。
    private(set) var debugSubscribes = 0
    private(set) var debugUpdates = 0
    private(set) var debugEnded = ""
    /// 房间面板里的页面（ADR-0052）：列表为空时壳画大加号。每一批都带当前值。
    private(set) var panel: [PanelItem] = []
    /// 房间面板上次看的那一页（本机记住，不同步）。
    private(set) var panelSelected: TabRef?
    /// 这条时间线是旁聊时给标题和状态（ADR-0093），主时间线为空。
    private(set) var sideChat: SideChatCard?
    /// 顶栏的 widget 入口和房间里有没有人在通话（ADR-0096）。
    private(set) var widgets: [RoomWidget] = []
    private(set) var callActive = false
    /// 群里在场的 AI（本账号认识的 Bot 的 Matrix 用户 ID）；私聊不算。nil 是成员名单还没取全、
    /// 在场的算不准，别下“群里没有 AI”的结论；空列表才是确定没有。
    private(set) var roomBots: [String]?
    /// 输入框正在编辑哪条（ADR-0109 第 10 节）；不在编辑时为空。
    private(set) var editing: ComposerEdit?
    /// 正在输入的人的显示名，不含自己（ADR-0109）。
    private(set) var typing: [String] = []
    /// 房间里各 Bot 的斜杠命令和被截掉的条数（ADR-0054、ADR-0109）：空的时候没有“菜单”按钮，敲“/”也不弹列表。
    private(set) var commands: [RoomCommand] = []
    private(set) var commandsOmitted: UInt32 = 0
    /// 跳到原消息的进展和“向下”按钮先回到的那一条（ADR-0109 第 4 节）。
    private(set) var jump: JumpState = .idle
    private(set) var backTo: String?
    /// 核心要壳滚到的那一条；每来一次 `focusRevision` 加一，视图按它滚。
    private(set) var focus: TimelineFocus?
    private(set) var focusRevision = 0
    private(set) var failure: Problem?
    /// 打不开时点“重试”加一，视图按它重新订阅。
    var attempt = 0

    private let core: ClientCore
    private var subscription: TimelineSubscription?
    let roomId: String
    /// 旁聊根的事件 ID；空是房间的主时间线（ADR-0093）。
    let threadRoot: String?

    init(core: ClientCore, roomId: String, threadRoot: String? = nil) {
        self.core = core
        self.roomId = roomId
        self.threadRoot = threadRoot
    }

    /// 在视图的 `.task` 里调用：订阅时间线，视图消失时订阅随之结束。
    func observe(pin: BottomPin) async -> Bool {
        failure = nil
        debugSubscribes += 1
        let subscription: TimelineSubscription
        do {
            subscription = try await core.subscribeTimeline(
                target: TimelineTarget(roomId: roomId, threadRoot: threadRoot)
            )
        } catch {
            failure = error.problem
            return false
        }
        self.subscription = subscription
        let state = subscription.debugStatus()
        let startLine = "observe start room=\(self.roomId) n=\(self.debugSubscribes) status=\(state)"
        walkLog.notice("\(startLine, privacy: .public)")
        defer {
            subscription.cancel()
            if self.subscription === subscription { self.subscription = nil }
        }
        guard !Task.isCancelled else { return false }
        for await update in subscription.updates() {
            guard !Task.isCancelled else { break }
            debugUpdates += 1
            walkLog.notice(
                "update diffs=\(update.diffs.count, privacy: .public) have=\(self.items.count, privacy: .public)"
            )
            apply(update, pin: pin)
        }
        // 走查：订阅结束的原因（取消还是自己断了），挂在 `ScrollDebug` 里。
        debugEnded = Task.isCancelled ? "cancelled" : "ended"
        // 状态在结束时重取：用开始时的快照会和同一行的 cancelled、上面的 debugEnded 对不上。
        let endLine = "timeline ended cancelled=\(Task.isCancelled) room=\(self.roomId)"
            + " status=\(subscription.debugStatus())"
        walkLog.notice("\(endLine, privacy: .public)")
        if !Task.isCancelled {
            failure = Problem(title: String(localized: .Problem1.problemGenericTitle))
        }
        // 先不弹回列表：页留着才能读到 `roomScroll` 里的原因。
        return !Task.isCancelled
    }

    func setScroll(_ scroll: TimelineScroll) {
        walkLog.notice("report \(String(describing: scroll), privacy: .public)")
        subscription?.setScroll(scroll: scroll)
    }

    private func apply(_ update: TimelineUpdate, pin: BottomPin) {
        // 这一批要滚到别处时不保留更新前的位置：保留会把跳转拉回去。
        if update.focus == nil, !prepends(update.diffs) { pin.prepareForUpdate() }
        if update.scrollToBottom { pin.pin() }
        for diff in update.diffs {
            apply(diff)
        }
        editing = update.editing
        backPagination = update.backPagination
        confirmations = update.confirmations
        viewport = update.viewport
        panel = update.panel
        panelSelected = update.panelSelected
        notice = update.keyNotice
        sideChat = update.sideChat
        widgets = update.widgets
        callActive = update.callActive
        roomBots = update.roomBots
        typing = update.typing
        commands = update.commands
        commandsOmitted = update.commandsOmitted
        jump = update.jump
        backTo = update.backTo
        if let focus = update.focus {
            self.focus = focus
            focusRevision += 1
        }
        if let anchor = update.anchor { self.anchor = anchor }
        if isLoaded { pin.stick() }
        isLoaded = true
    }

    // 翻页由房间锚点恢复处理，不应触发实时追加的底部跟随。
    private func prepends(_ diffs: [TimelineDiff]) -> Bool {
        diffs.contains { diff in
            switch diff {
            case .pushFront, .clear, .reset: true
            case let .insert(index, _): index == 0
            default: false
            }
        }
    }

    private func apply(_ diff: TimelineDiff) {
        switch diff {
        case let .append(more):
            items.append(contentsOf: more)
        case .clear:
            items.removeAll()
            generation += 1
        case let .pushFront(item):
            items.insert(item, at: 0)
        case let .pushBack(item):
            items.append(item)
        case .popFront:
            items.removeFirst()
        case .popBack:
            items.removeLast()
        case let .insert(index, item):
            items.insert(item, at: Int(index))
        case let .set(index, item):
            items[Int(index)] = item
        case let .remove(index):
            items.remove(at: Int(index))
        case let .truncate(length):
            items.removeSubrange(Int(length)...)
        case let .reset(all):
            items = all
        }
    }
}
