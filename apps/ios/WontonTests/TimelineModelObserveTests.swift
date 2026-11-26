import XCTest
import WontonClient
@testable import Wonton

/// `TimelineModel.observe` 的返回值决定 `ChatRoomView` 收不收页面：订阅自己结束（房间关了、旁聊根没了）要回
/// `true`，视图自己离开把任务取消时回 `false`。假核心照真核心的结束方式把发送端丢掉。
@MainActor
final class TimelineModelObserveTests: XCTestCase {
    func testObserveReturnsTrueWhenTheSubscriptionEnds() async throws {
        let core = try makeCore()
        let model = TimelineModel(core: core, roomId: "!ui-fake:home.example")
        let observing = Task { await model.observe(pin: BottomPin()) }
        let loaded = await waitUntil { model.isLoaded }
        XCTAssertTrue(loaded, "时间线没来第一批")

        XCTAssertTrue(
            core.uiFakeEndTimeline(target: TimelineTarget(roomId: model.roomId, threadRoot: nil)),
            "没有这条订阅"
        )

        let ended = await observing.value
        XCTAssertTrue(ended, "订阅自己结束要回 true，旁聊盖层和主对话才有机会收掉")
    }

    func testObserveReturnsFalseWhenCancelled() async throws {
        let core = try makeCore()
        let model = TimelineModel(core: core, roomId: "!ui-fake:home.example")
        let observing = Task { await model.observe(pin: BottomPin()) }
        let loaded = await waitUntil { model.isLoaded }
        XCTAssertTrue(loaded, "时间线没来第一批")

        observing.cancel()

        let ended = await observing.value
        XCTAssertFalse(ended, "视图自己离开（任务取消）不算订阅结束，别收页面")
    }

    private func waitUntil(_ condition: @MainActor () -> Bool) async -> Bool {
        for _ in 0..<200 {
            if condition() { return true }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return false
    }

    private func makeCore() throws -> ClientCore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        // ClientCore 没有显式 shutdown，句柄要等核心析构才释放；收尾抢在释放前删目录可能报 513。
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return try ClientCore.newUiFake(dataDir: directory.path)
    }
}
