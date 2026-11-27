package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.BotAccess
import app.wonton.protocol.BotChatAvailability
import app.wonton.protocol.BotChatUnavailable
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 这个 Bot 现在不能聊的原因；能用、不是 Bot 或还没取到时为空，输入框照常。
 * 可达清单带原因时用 [BotAccess]；旧核心只有 [BotChatAvailability] 时按同一套原因退化。
 */
internal fun botAccessOf(availability: BotChatAvailability?): BotAccess? = when (availability) {
    null -> null
    BotChatAvailability.Available -> BotAccess.Contact
    BotChatAvailability.Unknown -> BotAccess.Unknown
    is BotChatAvailability.Unavailable -> BotAccess.Unusable(availability.reason)
}

/** 具体原因的一句话；不认识的原因退回“用不了”。 */
internal fun botAccessReasonNotice(reason: BotChatUnavailable): Int = when (reason) {
    BotChatUnavailable.OWNER_PRIVATE -> R.string.room_bot_owner_private
    BotChatUnavailable.GRANT_NOT_ACTIVE -> R.string.room_bot_grant_revoked
    BotChatUnavailable.ROLE_LACKS_SHARED_BOTS -> R.string.room_bot_role_lacks_shared_bots
    BotChatUnavailable.APP_NOT_SHARED -> R.string.room_bot_app_not_shared
    BotChatUnavailable.CONTACT_PENDING -> R.string.room_bot_contact_pending
    BotChatUnavailable.CONTACT_DECLINED -> R.string.room_bot_contact_declined
    BotChatUnavailable.BOT_MISSING -> R.string.room_bot_missing
    BotChatUnavailable.OTHER -> R.string.room_bot_unavailable
}

/** 这个 Bot 现在不能聊的原因；能用、不是 Bot 或还没取到时为空，输入框照常。 */
internal fun botAccessNotice(access: BotAccess?): Int? = when (access) {
    null, BotAccess.Own, BotAccess.Shared, BotAccess.Contact -> null
    BotAccess.Unknown -> R.string.room_bot_unknown
    is BotAccess.Unusable -> botAccessReasonNotice(access.reason)
}

/** 确定不能用时命令菜单和旁聊照片直发要说的那句；能用或资格未知（照常发）时为空。 */
internal fun botAccessBlockedProblem(canSend: Boolean, notice: Int?): Problem? =
    if (canSend) null else notice?.let { Problem(titleRes = it) }

/** 不能聊时输入框上方的一句话；房间和历史照留，只是不发。 */
@Composable
internal fun BotChatNoticeRow(text: Int) {
    Text(
        stringResource(text),
        Modifier.fillMaxWidth().padding(
            horizontal = WontonTheme.spacing.pageInset,
            vertical = WontonTheme.spacing.s4,
        ),
        style = WontonTheme.typography.body,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

/** 转发能去的房间：别的、已经加入、能发的房间。 */
internal fun RoomRow.forwardTarget(roomId: String): Boolean =
    this.roomId != roomId && membership == RoomMembership.JOINED && !isReadOnly
