package app.wonton.ui

import app.wonton.protocol.BotAccess
import app.wonton.protocol.BotChatAvailability
import app.wonton.protocol.BotChatUnavailable
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/// 房间提示的规矩：能用的不吵，确定不能用的照实说原因；旧核心只给“能不能”时按同一套原因退化。
class RoomScreenNoticesTest {
    @Test
    fun usableAccessShowsNoNotice() {
        assertNull(botAccessNotice(null))
        assertNull(botAccessNotice(BotAccess.Own))
        assertNull(botAccessNotice(BotAccess.Shared))
        assertNull(botAccessNotice(BotAccess.Contact))
        assertNull(botAccessNotice(BotAccess.Unknown))
    }

    @Test
    fun everyKnownReasonKeepsItsOwnSentence() {
        val expected = listOf(
            BotChatUnavailable.OWNER_PRIVATE to R.string.room_bot_owner_private,
            BotChatUnavailable.GRANT_NOT_ACTIVE to R.string.room_bot_grant_revoked,
            BotChatUnavailable.ROLE_LACKS_SHARED_BOTS to R.string.room_bot_role_lacks_shared_bots,
            BotChatUnavailable.APP_NOT_SHARED to R.string.room_bot_app_not_shared,
            BotChatUnavailable.CONTACT_PENDING to R.string.room_bot_contact_pending,
            BotChatUnavailable.CONTACT_DECLINED to R.string.room_bot_contact_declined,
            BotChatUnavailable.BOT_MISSING to R.string.room_bot_missing,
            BotChatUnavailable.OTHER to R.string.room_bot_unavailable,
        )
        for ((reason, sentence) in expected) {
            assertEquals("reason $reason", sentence, botAccessNotice(BotAccess.Unusable(reason)))
        }
    }

    @Test
    fun oldCoreAvailabilityFallsBackToTheSameReasons() {
        assertEquals(BotAccess.Contact, botAccessOf(BotChatAvailability.Available))
        assertEquals(BotAccess.Unknown, botAccessOf(BotChatAvailability.Unknown))
        // 旧核心给的具体原因原样带过来，不被吞成“不知道”。
        assertEquals(
            R.string.room_bot_owner_private,
            botAccessNotice(botAccessOf(BotChatAvailability.Unavailable(BotChatUnavailable.OWNER_PRIVATE))),
        )
        // 不认识的原因仍有通用的“用不了”，不能没有提示。
        assertEquals(
            R.string.room_bot_unavailable,
            botAccessNotice(botAccessOf(BotChatAvailability.Unavailable(BotChatUnavailable.OTHER))),
        )
    }

    @Test
    fun blockedProblemOnlyCoversDefiniteUnusable() {
        assertNull(botAccessBlockedProblem(canSend = true, notice = R.string.room_bot_unavailable))
        assertNull(botAccessBlockedProblem(canSend = false, notice = null))
        assertEquals(
            R.string.room_bot_contact_pending,
            botAccessBlockedProblem(canSend = false, notice = R.string.room_bot_contact_pending)?.titleRes,
        )
    }
}
