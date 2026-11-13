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
    /// 房间面板里的页面（ADR-0052）：列表为空时壳画大加号。每一批都带当前值。
    private(set) var panel: [PanelItem] = []
    /// 房间面板上次看的那一页（本机记住，不同步）。
    private(set) var panelSelected: TabRef?
    /// 房间里盖着消息列表的东西开着：上推拉面板的入口不响应。核心合成，壳只读。
    private(set) var entryBlocked = false
    /// 这条时间线是旁聊时给标题和状态（ADR-0093），主时间线为空。
    private(set) var sideChat: SideChatCard?
    private(set) var failure: Problem?
    /// 打不开时点“重试”加一，视图按它重新订阅。
    var attempt = 0

    private let core: ClientCore
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
        let subscription: TimelineSubscription
        do {
            subscription = try await core.subscribeTimeline(
                target: TimelineTarget(roomId: roomId, threadRoot: threadRoot)
            )
        } catch {
            failure = error.problem
            return false
        }
        defer { subscription.cancel() }
        guard !Task.isCancelled else { return false }
        for await update in subscription.updates() {
            guard !Task.isCancelled else { break }
            // 翻页由房间的锚点恢复处理；仅保留不改变开头的实时追加和编辑。
            let prepends = update.diffs.contains { diff in
                switch diff {
                case .pushFront, .clear, .reset: true
                case let .insert(index, _): index == 0
                default: false
                }
            }
            if !prepends { pin.prepareForUpdate() }
            if update.scrollToBottom { pin.pin() }
            for diff in update.diffs {
                apply(diff)
            }
            backPagination = update.backPagination
            confirmations = update.confirmations
            viewport = update.viewport
            panel = update.panel
            panelSelected = update.panelSelected
            entryBlocked = update.entryBlocked
            confirmations = update.confirmations
            notice = update.keyNotice
            sideChat = update.sideChat
            if let anchor = update.anchor {
                self.anchor = anchor
            }
            if isLoaded { pin.stick() }
            isLoaded = true
        }
        return !Task.isCancelled
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
