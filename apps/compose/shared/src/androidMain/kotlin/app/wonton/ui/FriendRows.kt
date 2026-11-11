package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.Action
import app.wonton.protocol.FriendBlock
import app.wonton.protocol.FriendBlockScope
import app.wonton.protocol.FriendBlockTarget
import app.wonton.protocol.FriendGroup
import app.wonton.protocol.FriendRow
import app.wonton.protocol.FriendsState
import app.wonton.protocol.FriendsViewModel
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding

@Composable
internal fun FriendList(
    friends: FriendsViewModel,
    failed: FriendsState.Failed?,
    onRetry: () -> Unit,
    onAction: (Action) -> Unit,
    onAdd: () -> Unit,
    onMenu: (FriendRow) -> Unit,
    onNewGroup: () -> Unit,
    onRenameGroup: (FriendGroup) -> Unit,
    onDeleteGroup: (FriendGroup) -> Unit,
    onBlockServer: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        failed?.let { ProblemRow(it.problem(), onRetry) }
        FriendRequestRows(friends, onAction)
        FriendRows(friends, onMenu, onAdd)
        FriendGroups(friends, onNewGroup, onRenameGroup, onDeleteGroup)
        BlockedSection(friends.blocks(), friends.canBlockFederation, onBlockServer) {
            onAction(Action.UnblockFriend(it.target, it.scope))
        }
    }
}

@Composable
private fun FriendRequestRows(friends: FriendsViewModel, onAction: (Action) -> Unit) {
    if (friends.incoming.isEmpty() && friends.outgoing.isEmpty()) return
    FriendSection(stringResource(R.string.friends_requests_title)) {
        WontonGroup {
            friends.incoming.forEachIndexed { index, row ->
                if (index > 0) WontonSeparator()
                IncomingFriendRequestRow(
                    row,
                    { onAction(Action.ApproveFriendRequest(row.roomId)) },
                    { onAction(Action.RejectFriendRequest(row.roomId)) },
                )
            }
            friends.outgoing.forEachIndexed { index, row ->
                if (friends.incoming.isNotEmpty() || index > 0) WontonSeparator()
                OutgoingFriendRequestRow(row) { onAction(Action.CancelFriendRequest(row.roomId)) }
            }
        }
    }
}

@Composable
private fun FriendRows(friends: FriendsViewModel, onMenu: (FriendRow) -> Unit, onAdd: () -> Unit) {
    if (friends.friends.isEmpty()) return
    FriendSection(stringResource(R.string.friends_list_title)) {
        val names = friends.groups.associate { it.id to it.name }
        WontonGroup {
            friends.friends.forEachIndexed { index, row ->
                if (index > 0) WontonSeparator()
                FriendRowView(row, names) { onMenu(row) }
            }
        }
        WontonRow(stringResource(R.string.friends_add), onClick = onAdd)
    }
}

@Composable
private fun FriendGroups(
    friends: FriendsViewModel,
    onNew: () -> Unit,
    onRename: (FriendGroup) -> Unit,
    onDelete: (FriendGroup) -> Unit,
) {
    FriendSection(stringResource(R.string.friends_groups_title)) {
        WontonGroup {
            friends.groups.forEachIndexed { index, group ->
                if (index > 0) WontonSeparator()
                GroupRow(group, { onRename(group) }, { onDelete(group) })
            }
            if (friends.groups.isNotEmpty()) WontonSeparator()
            WontonRow(
                stringResource(R.string.friends_group_new),
                onClick = onNew,
                titleTint = WontonTheme.colors.primary,
            )
        }
    }
}

@Composable
private fun FriendSection(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(title, style = WontonTheme.typography.sectionTitle, color = WontonTheme.colors.textPrimary.toColor())
        content()
    }
}

@Composable
private fun FriendRowView(row: FriendRow, groupNames: Map<String, String>, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val groups = row.groups.mapNotNull { groupNames[it] }
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(row.name(), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            Text(
                groups.takeIf { it.isNotEmpty() }?.joinToString(stringResource(R.string.friends_group_separator))
                    ?: stringResource(R.string.friends_no_group),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
            Text(
                pluralStringResource(
                    R.plurals.friends_shared_plugins,
                    row.sharedPlugins.size,
                    row.sharedPlugins.size,
                ),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
private fun GroupRow(group: FriendGroup, onRename: () -> Unit, onDelete: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(
            group.name,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonSmallButton(stringResource(R.string.common_rename), onRename)
        WontonSmallButton(stringResource(R.string.common_delete), onDelete, danger = true)
    }
}

@Composable
private fun BlockedSection(
    blocks: List<FriendBlock>,
    canBlockFederation: Boolean,
    onBlockServer: () -> Unit,
    onUnblock: (FriendBlock) -> Unit,
) {
    if (blocks.isEmpty() && !canBlockFederation) return
    FriendSection(stringResource(R.string.friends_blocked_title)) {
        WontonGroup {
            blocks.forEachIndexed { index, block ->
                if (index > 0) WontonSeparator()
                BlockRow(block) { onUnblock(block) }
            }
            if (canBlockFederation) {
                if (blocks.isNotEmpty()) WontonSeparator()
                WontonRow(stringResource(R.string.friends_block_server), onClick = onBlockServer)
            }
        }
    }
}

@Composable
private fun BlockRow(block: FriendBlock, onUnblock: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                block.target.text(),
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = colors.textPrimary.toColor(),
            )
            Text(
                when (block.scope) {
                    FriendBlockScope.INVITATIONS -> stringResource(R.string.friends_block_invitations)
                    FriendBlockScope.FEDERATION -> stringResource(R.string.friends_block_federation)
                    FriendBlockScope.OTHER -> stringResource(R.string.friends_block_unknown)
                },
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
        WontonSmallButton(stringResource(R.string.friends_unblock), onUnblock)
    }
}

internal fun FriendBlockTarget.text(): String = when (this) {
    is FriendBlockTarget.User -> userId
    is FriendBlockTarget.Server -> serverName
    FriendBlockTarget.Other -> toString()
}

@Composable
internal fun FriendEmpty(onAdd: () -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.friends_empty),
            style = WontonTheme.typography.body.copy(textAlign = TextAlign.Center),
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        WontonButton(stringResource(R.string.friends_add), onAdd, kind = WontonButtonKind.SECONDARY)
    }
}
