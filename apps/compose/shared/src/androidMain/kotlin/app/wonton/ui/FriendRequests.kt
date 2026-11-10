package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import app.wonton.protocol.FriendRequest
import app.wonton.protocol.FriendRequestKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

/**
 * 一条收到的私聊邀请：带 `app.wonton.friend` 的标成好友请求，普通邀请不加标签（ADR-0051）。
 * 请求认房间，不认显示名。
 */
@Composable
internal fun IncomingFriendRequestRow(row: FriendRequest, onApprove: () -> Unit, onReject: () -> Unit) {
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    Column(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            Text(
                row.displayName?.takeIf { it.isNotBlank() } ?: row.userId,
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
            )
            if (row.kind == FriendRequestKind.FRIEND) {
                Text(
                    stringResource(R.string.friends_request_label),
                    style = WontonTheme.typography.supporting,
                    color = colors.primary.toColor(),
                )
            }
        }
        Text(
            row.userId,
            style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
            color = secondary,
        )
        Text(RelativeTime(row.invitedAtMs.toLong()), style = WontonTheme.typography.supporting, color = secondary)
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4, Alignment.End),
        ) {
            // 删除拒绝、勾同意；命中区都是 48，图标比文字好认。
            WontonIconButton(
                WontonIcon.Trash,
                stringResource(R.string.friends_request_reject),
                onReject,
                WontonTheme.colors.danger.toColor(),
            )
            WontonIconButton(WontonIcon.Check, stringResource(R.string.friends_request_approve), onApprove)
        }
    }
}

/** 一条自己发出、对方还没处理的请求：说清在等，能取消。 */
@Composable
internal fun OutgoingFriendRequestRow(row: FriendRequest, onCancel: () -> Unit) {
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(row.displayName?.takeIf { it.isNotBlank() } ?: row.userId, style = WontonTheme.typography.body)
            Text(
                stringResource(R.string.friends_request_waiting),
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
        }
        WontonSmallButton(stringResource(R.string.friends_request_cancel), onCancel)
    }
}

/**
 * 配对页最上面的好友请求（ADR-0051）：好友请求和普通私聊邀请都在这儿处理，不混进聊天列表。
 * 一列表格，请求为空时整节不占位置。
 */
@Composable
internal fun FriendRequestsSection(
    requests: List<FriendRequest>,
    onApprove: (FriendRequest) -> Unit,
    onReject: (FriendRequest) -> Unit,
) {
    if (requests.isEmpty()) return
    PeerSection {
        Text(
            stringResource(R.string.friends_peers_title),
            style = WontonTheme.typography.sectionTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonGroup {
            requests.forEachIndexed { index, row ->
                if (index > 0) WontonSeparator()
                IncomingFriendRequestRow(row, { onApprove(row) }, { onReject(row) })
            }
        }
    }
}
