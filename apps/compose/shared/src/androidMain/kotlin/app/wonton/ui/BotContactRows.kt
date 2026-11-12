package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import app.wonton.protocol.BotContact
import app.wonton.protocol.BotContactFailure
import app.wonton.protocol.BotContactsState
import app.wonton.protocol.ContactError
import app.wonton.protocol.ContactKind
import app.wonton.protocol.ContactState
import app.wonton.protocol.PeerStatus
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

/** 显示名；没有时用 Matrix 用户 ID。 */
internal fun BotContact.name(): String = displayName?.takeIf { it.isNotBlank() } ?: userId

/** 发来的请求：说清同意意味着什么，同意或拒绝。 */
@Composable
internal fun IncomingContactRow(contact: BotContact, onApprove: () -> Unit, onReject: () -> Unit) {
    ContactRowFrame(contact) {
        Text(
            stringResource(R.string.contacts_incoming_warning),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.warning.toColor(),
        )
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.End),
        ) {
            WontonSmallButton(stringResource(R.string.contacts_reject), onReject, danger = true)
            WontonSmallButton(stringResource(R.string.contacts_approve), onApprove)
        }
    }
}

/** 双方都同意了的联系人：所在的节已经说明生效，行上不再写状态；跨机的说明消息为什么过不去。 */
@Composable
internal fun ActiveContactRow(contact: BotContact, onRemove: () -> Unit) {
    ContactRowFrame(
        contact,
        trailing = {
            WontonIconButton(
                WontonIcon.Trash,
                stringResource(R.string.contacts_remove),
                onRemove,
                WontonTheme.colors.danger.toColor(),
            )
        },
    )
}

/** 自己发出、还没生效的：等对方、对方拒绝了、没有这个人。等对方的可撤销，其余的可删掉这一行。 */
@Composable
internal fun SentContactRow(contact: BotContact, onRemove: () -> Unit) {
    ContactRowFrame(
        contact,
        trailing = {
            val waiting = contact.state == ContactState.PENDING
            WontonSmallButton(
                stringResource(if (waiting) R.string.contacts_withdraw else R.string.contacts_clear),
                onRemove,
                danger = !waiting,
            )
        },
    ) {
        Text(
            stringResource(contact.statusText()),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 一行的骨架：名字、谁的 Bot、用户 ID、跨机时的连接情况，下面是各状态自己的内容。 */
@Composable
private fun ContactRowFrame(
    contact: BotContact,
    trailing: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit = {},
) {
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(contact.name(), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            contact.keeperDisplayName?.takeIf { it.isNotBlank() }?.let {
                Text(
                    stringResource(R.string.contacts_keeper, it),
                    style = WontonTheme.typography.supporting,
                    color = secondary,
                )
            }
            Text(
                contact.userId,
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = secondary,
            )
            contact.remoteProblem()?.let {
                Text(stringResource(it), style = WontonTheme.typography.supporting, color = colors.warning.toColor())
            }
            content()
        }
        trailing?.invoke()
    }
}

/** 还没生效的一行写什么：等对方时按对方是 Bot 还是人选“同意”或“接受”（ADR-0050）。 */
internal fun BotContact.statusText(): Int = when (state) {
    ContactState.PENDING -> if (kind == ContactKind.PERSON) {
        R.string.contacts_status_waiting_person
    } else {
        R.string.contacts_status_waiting_bot
    }
    ContactState.DECLINED -> R.string.contacts_status_declined
    ContactState.NOT_FOUND -> R.string.contacts_status_not_found
    ContactState.INCOMING, ContactState.ACTIVE, ContactState.OTHER -> R.string.contacts_status_unknown
}

/** 跨机的联系人消息过不去的原因，按那台服务器的配对状态写；同机的、连得上的没有。 */
internal fun BotContact.remoteProblem(): Int? {
    val remote = remote ?: return null
    return when (remote.peerStatus) {
        null -> R.string.contacts_remote_unpaired
        PeerStatus.OFFLINE -> R.string.peers_status_offline
        PeerStatus.REMOVED_BY_PEER -> R.string.peers_status_removed
        PeerStatus.PUBLIC_ACCESS_OFF -> R.string.peers_status_public_access_off
        PeerStatus.CONNECTED, PeerStatus.CHECKING, PeerStatus.AWAITING_APPROVAL, PeerStatus.DECLINED,
        PeerStatus.OTHER -> null
    }
}

fun BotContactsState.Failed.problem(): Problem = contactProblem(error, message)

fun BotContactFailure.problem(): Problem = contactProblem(error, message)

/** 联系人接口失败的一句话，按核心给的类别写（ADR-0043）；核心写好的说明收进“详情”。不认识的类别用通用的一句。 */
private fun contactProblem(error: ContactError, message: String): Problem = Problem(
    titleRes = when (error) {
        ContactError.INVALID_USER_ID -> R.string.contacts_error_invalid_user_id
        ContactError.NOT_PAIRED -> R.string.contacts_error_not_paired
        ContactError.ALREADY_CONTACT -> R.string.contacts_error_already_contact
        ContactError.BOT_NOT_FOUND -> R.string.contacts_error_bot_not_found
        ContactError.CONTACT_NOT_FOUND -> R.string.contacts_error_contact_not_found
        ContactError.NOT_KEEPER -> R.string.contacts_error_not_keeper
        ContactError.PEER_UNREACHABLE -> R.string.contacts_error_peer_unreachable
        ContactError.UNAUTHENTICATED -> R.string.contacts_error_unauthenticated
        ContactError.UPSTREAM -> R.string.contacts_error_upstream
        ContactError.TRANSPORT_UNAVAILABLE -> R.string.peers_error_transport
        ContactError.PROTOCOL, ContactError.OTHER -> R.string.peers_error_other
    },
    nextStepRes = R.string.contacts_error_not_paired_next.takeIf { error == ContactError.NOT_PAIRED },
    detail = message,
)
