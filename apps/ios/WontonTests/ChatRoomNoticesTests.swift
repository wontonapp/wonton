import XCTest
import WontonClient
@testable import Wonton

/// 房间提示的规矩：能用的不吵，确定不能用的按原因给专属句，不认识的原因退回通用句。
final class ChatRoomNoticesTests: XCTestCase {
    func testUsableAccessShowsNoNotice() {
        XCTAssertNil(botAccessNoticeText(.own))
        XCTAssertNil(botAccessNoticeText(.shared))
        XCTAssertNil(botAccessNoticeText(.contact))
    }

    func testUnknownAccessStillWarns() {
        XCTAssertEqual(botAccessNoticeText(.unknown)?.key, "room.botUnknown")
    }

    func testEveryKnownReasonKeepsItsOwnSentence() {
        let expected: [(BotChatUnavailable, String)] = [
            (.ownerPrivate, "room.botOwnerPrivate"),
            (.grantNotActive, "room.botGrantRevoked"),
            (.roleLacksSharedBots, "room.botRoleLacksSharedBots"),
            (.appNotShared, "room.botAppNotShared"),
            (.contactPending, "room.botContactPending"),
            (.contactDeclined, "room.botContactDeclined"),
            (.botMissing, "room.botMissing"),
        ]
        for (reason, key) in expected {
            XCTAssertEqual(botAccessNoticeText(.unusable(reason: reason))?.key, key, "\(reason)")
        }
    }

    func testUnrecognizedReasonFallsBackToGeneric() {
        XCTAssertEqual(botAccessNoticeText(.unusable(reason: .other))?.key, "room.botUnavailable")
    }
}
