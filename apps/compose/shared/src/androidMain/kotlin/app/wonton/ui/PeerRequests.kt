package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import app.wonton.protocol.PeerRequestRow
import app.wonton.protocol.RejectedPeerRow
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon

private const val EXPANDED_DEGREES = 90f

/** 一节内容：小标题在组上面。 */
@Composable
internal fun PeerSection(content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) { content() }
}

/** 发来的一条配对请求：除了 id 都是对方自报的（ADR-0050）；和已配对的机器重名时醒目标出。点开展开详情。 */
@Composable
internal fun PeerRequestRowView(row: PeerRequestRow, onApprove: () -> Unit, onReject: () -> Unit) {
    var expanded by remember(row.id) { mutableStateOf(false) }
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    Column(
        Modifier.fillMaxWidth().clickable(role = Role.Button) { expanded = !expanded }
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Text(row.ownerDisplayName, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        row.conflictsWith?.let {
            Text(
                stringResource(R.string.peers_request_conflicts, it),
                style = WontonTheme.typography.supporting,
                color = colors.warning.toColor(),
            )
        }
        Text(
            row.serverName,
            style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
            color = secondary,
        )
        Text(RelativeTime(row.receivedAtMs.toLong()), style = WontonTheme.typography.supporting, color = secondary)
        if (expanded) {
            row.detail?.let { Text(it, style = WontonTheme.typography.supporting, color = secondary) }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.End),
        ) {
            WontonSmallButton(stringResource(R.string.peers_request_reject), onReject, danger = true)
            WontonSmallButton(stringResource(R.string.peers_request_approve), onApprove)
        }
    }
}

/** 底下默认收起的“已拒绝”：都是主人自己点错的，能撤销，撤销后对方再发就照常收下（ADR-0050）。 */
@Composable
internal fun RejectedSection(rows: List<RejectedPeerRow>, onUnreject: (String) -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val colors = WontonTheme.colors
    PeerSection {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                .clickable(role = Role.Button) { expanded = !expanded },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            Text(
                stringResource(R.string.peers_rejected_title),
                Modifier.weight(1f),
                style = WontonTheme.typography.sectionTitle,
                color = colors.textPrimary.toColor(),
            )
            Icon(
                WontonIcon.Forward,
                // 装饰：整行可点，箭头只表示能展开。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall).rotate(if (expanded) EXPANDED_DEGREES else 0f),
                tint = colors.textSecondary.toColor(),
            )
        }
        if (expanded) {
            WontonGroup {
                rows.forEachIndexed { index, row ->
                    if (index > 0) WontonSeparator()
                    RejectedRowView(row, onUnreject = { onUnreject(row.id) })
                }
            }
        }
    }
}

/** 拒绝过的一条：对方自报的名字和服务器，加撤销。 */
@Composable
private fun RejectedRowView(row: RejectedPeerRow, onUnreject: () -> Unit) {
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(row.ownerDisplayName, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            Text(
                row.serverName,
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = secondary,
            )
            Text(RelativeTime(row.rejectedAtMs.toLong()), style = WontonTheme.typography.supporting, color = secondary)
        }
        WontonSmallButton(stringResource(R.string.peers_rejected_undo), onUnreject)
    }
}
