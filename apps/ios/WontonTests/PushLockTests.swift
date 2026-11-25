import XCTest

@testable import Wonton

/// 锁定标记的语义（ADR-0102 §4）：没写过算没锁，读不懂算锁着，宁可少露内容。
final class PushLockTests: XCTestCase {
    func testNoMarkerBecauseTheLockWasNeverOnReadsAsUnlocked() {
        XCTAssertFalse(PushLock.locked(marker: nil))
    }

    func testCoreWrittenMarkersAreFollowed() {
        XCTAssertTrue(PushLock.locked(marker: Data(#"{"locked":true}"#.utf8)))
        XCTAssertFalse(PushLock.locked(marker: Data(#"{"locked":false}"#.utf8)))
    }

    func testAnUnreadableMarkerHidesContent() {
        XCTAssertTrue(PushLock.locked(marker: Data("{".utf8)))
        XCTAssertTrue(PushLock.locked(marker: Data()))
    }
}
