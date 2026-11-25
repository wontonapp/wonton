import XCTest

@testable import Wonton

/// 锁定标记的语义（ADR-0102 §4）：没写过算没锁；读不懂、缺字段算锁着；宽限按截止时刻判。
final class PushLockTests: XCTestCase {
    private static let noPendingLock = #"{"locked":false,"locks_at_ms":18446744073709551615}"#

    func testNoMarkerBecauseTheLockWasNeverOnReadsAsUnlocked() {
        XCTAssertFalse(PushLock.locked(marker: nil))
    }

    func testCoreWrittenMarkersAreFollowed() {
        XCTAssertTrue(PushLock.locked(marker: Data(#"{"locked":true,"locks_at_ms":0}"#.utf8)))
        XCTAssertFalse(PushLock.locked(marker: Data(Self.noPendingLock.utf8)))
    }

    func testAPendingDeadlineLocksOnlyOnceItPasses() {
        let pending = Data(#"{"locked":false,"locks_at_ms":2000}"#.utf8)
        XCTAssertFalse(PushLock.locked(marker: pending, now: 1_000))
        XCTAssertTrue(PushLock.locked(marker: pending, now: 2_000))
        XCTAssertTrue(PushLock.locked(marker: pending, now: 3_000))
    }

    func testAMarkerWithoutDeadlineHidesContent() {
        XCTAssertTrue(PushLock.locked(marker: Data(#"{"locked":false}"#.utf8)))
        XCTAssertTrue(PushLock.locked(marker: Data(#"{"locked":true}"#.utf8)))
    }

    func testAnUnreadableMarkerHidesContent() {
        XCTAssertTrue(PushLock.locked(marker: Data("{".utf8)))
        XCTAssertTrue(PushLock.locked(marker: Data()))
    }
}
