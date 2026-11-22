package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.R
import app.wonton.protocol.BotChatAvailability
import app.wonton.protocol.BotChatUnavailable

/** 这个 Bot 现在不能聊的原因；能用、不是 Bot 或还没取到时为空，输入框照常。 */
internal fun botChatNotice(availability: BotChatAvailability?): Int? = when (availability) {
    null, BotChatAvailability.Available -> null
    is BotChatAvailability.Unavailable -> when (availability.reason) {
        BotChatUnavailable.OTHER_PRIVATE_BOT -> R.string.room_bot_other_private
        BotChatUnavailable.GRANT_NOT_ACTIVE -> R.string.room_bot_grant_revoked
        BotChatUnavailable.OTHER -> R.string.room_bot_unavailable
    }
    BotChatAvailability.Unknown -> R.string.room_bot_unknown
}

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
