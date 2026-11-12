import Observation
import WontonClient

@MainActor
@Observable
final class TimelineModel {
    private(set) var items: [TimelineItem] = []
    private(set) var backPagination: BackPagination = .idle
    /// 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。
    private(set) var isLoaded = false
    private(set) var failure: Problem?
    /// 打不开时点“重试”加一，视图按它重新订阅。
    var attempt = 0

    private let core: ClientCore
    let roomId: String

    init(core: ClientCore, roomId: String) {
        self.core = core
        self.roomId = roomId
    }

    /// 在视图的 `.task` 里调用：订阅时间线，视图消失时订阅随之结束。
    func observe() async {
        failure = nil
        let subscription: TimelineSubscription
        do {
            subscription = try await core.subscribeTimeline(roomId: roomId)
        } catch {
            failure = error.problem
            return
        }
        for await update in subscription.updates() {
            for diff in update.diffs {
                apply(diff)
            }
            backPagination = update.backPagination
            isLoaded = true
        }
    }

    private func apply(_ diff: TimelineDiff) {
        switch diff {
        case let .append(more):
            items.append(contentsOf: more)
        case .clear:
            items.removeAll()
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
