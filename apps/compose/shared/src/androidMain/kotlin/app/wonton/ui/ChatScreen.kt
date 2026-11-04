package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatSync
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Connection
import app.wonton.ui.icons.LockFill
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
fun ChatRoute(core: ClientCore, navigation: ChatNavigation) {
    val updates = remember(core) { core.chatUpdates() }
    val initial = remember(core) { core.chat() }
    val viewModel by updates.collectAsState(initial = initial)
    var rejection by remember { mutableStateOf<String?>(null) }
    // 只在被拒绝时更新提示：成功的操作（比如自动发出的已读回执）不清掉还没看的拒绝原因。
    val onAction: (Action) -> Unit = { action -> core.send(action)?.let { rejection = it } }

    LaunchedEffect(viewModel) { navigation.onUpdate(viewModel) }

    val room = navigation.openRoom
    // 提示条和通知能在备份页面上打开房间。
    LaunchedEffect(room) { if (room != null) navigation.showingBackup = false }
    when {
        navigation.showingBackup -> {
            BackHandler { navigation.showingBackup = false }
            ChatBackupScreen(viewModel.encryption, onAction, onClose = { navigation.showingBackup = false })
        }
        navigation.startingChat -> {
            BackHandler { navigation.startingChat = false }
            StartChatScreen(
                onStart = { userId ->
                    navigation.startingChat = false
                    rejection = navigation.openDirectChat(core, userId)
                },
                onClose = { navigation.startingChat = false },
            )
        }
        room != null -> {
            BackHandler { navigation.openRoom = null }
            val row = viewModel.rooms.firstOrNull { it.roomId == room }
            RoomRoute(
                core = core,
                roomId = room,
                title = row?.name ?: room,
                isDirect = row?.isDirect == true,
                isReadOnly = row?.isReadOnly == true,
                onAction = onAction,
                onReject = { rejection = it },
                onClose = { navigation.openRoom = null },
            )
        }
        else -> ChatListScreen(
            viewModel = viewModel,
            onAction = onAction,
            onOpenRoom = { navigation.openRoom = it },
            onOpenBackup = { navigation.showingBackup = true },
            onStartChat = { navigation.startingChat = true },
        )
    }

    viewModel.problem?.let { WontonDialog("操作失败", it.message, { onAction(Action.DismissChatProblem) }) }
    rejection?.let { WontonDialog("操作被拒绝", it, { rejection = null }) }
}

@Composable
private fun ChatListScreen(
    viewModel: ChatViewModel,
    onAction: (Action) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenBackup: () -> Unit,
    onStartChat: () -> Unit,
) {
    WontonScreen(
        bar = {
            WontonRootBar("聊天") {
                WontonIconButton(WontonIcon.Compose, "发起私聊", onStartChat)
            }
        },
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = WontonTheme.spacing.s8)) {
            item(key = "notices") { Notices(viewModel, onOpenBackup) }
            itemsIndexed(viewModel.rooms, key = { _, row -> row.roomId }) { index, row ->
                if (index > 0) WontonSeparator()
                if (row.membership == RoomMembership.INVITED) {
                    InviteItem(row, onAction)
                } else {
                    RoomItem(row, onClick = { onOpenRoom(row.roomId) })
                }
            }
        }
        // 同步好之前列表是空的不代表没有聊天。
        val synced = viewModel.account is ChatAccount.SignedIn && viewModel.sync == ChatSync.Running
        if (viewModel.rooms.isEmpty() && synced) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    "还没有聊天",
                    style = WontonTheme.typography.itemTitle,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
            }
        }
    }
}

@Composable
private fun Notices(viewModel: ChatViewModel, onOpenBackup: () -> Unit) {
    val account = viewModel.account
    val sync = viewModel.sync
    val reminder = viewModel.encryption.backupReminder
    val failure = (account as? ChatAccount.Failed)?.message
        ?: (sync as? ChatSync.Failed)?.message.takeIf { account is ChatAccount.SignedIn }
    val progress = when {
        account == ChatAccount.SigningIn -> "正在登录…"
        account is ChatAccount.SignedIn && (sync == ChatSync.Idle || sync == ChatSync.Starting) -> "正在同步…"
        else -> null
    }
    if (account != ChatAccount.SignedOut && listOfNotNull(failure, progress, reminder).isEmpty()) return
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        if (account == ChatAccount.SignedOut) {
            WontonNotice(WontonIcon.Connection, "还没有连上家里的服务器", isError = false)
        }
        failure?.let { WontonNotice(WontonIcon.Warning, it) }
        progress?.let {
            Text(it, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
        }
        reminder?.let { WontonGroup { WontonRow(it, showsForward = true, onClick = onOpenBackup) } }
    }
}

@Composable
private fun RoomItem(row: RoomRow, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            ) {
                Text(
                    row.name,
                    Modifier.weight(1f, fill = false),
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (row.isEncrypted) {
                    Icon(
                        WontonIcon.LockFill,
                        contentDescription = "已加密",
                        modifier = Modifier.size(WontonTheme.size.iconTiny),
                        tint = colors.textSecondary.toColor(),
                    )
                }
            }
            row.preview?.let { preview ->
                Text(
                    if (preview.isOwn) "我：${preview.text}" else preview.text,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (row.unreadCount > 0uL) {
            val spoken = "${row.unreadCount} 条未读"
            Text(
                row.unreadCount.toString(),
                Modifier.semantics { contentDescription = spoken }
                    .accentFill(CircleShape)
                    .padding(horizontal = WontonTheme.spacing.s8, vertical = WontonTheme.spacing.s4),
                style = WontonTheme.typography.caption.copy(fontFeatureSettings = "tnum"),
                color = colors.accentInkOnFill.toColor(),
            )
        }
    }
}

@Composable
private fun InviteItem(row: RoomRow, onAction: (Action) -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            "${row.name} 邀请你${if (row.isDirect) "私聊" else "加入房间"}",
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton("接受", { onAction(Action.JoinRoom(row.roomId)) })
            WontonButton("拒绝", { onAction(Action.LeaveRoom(row.roomId)) }, kind = WontonButtonKind.SECONDARY)
        }
    }
}

/** 输入对方的 Matrix ID 发起私聊，房间出现后打开。 */
@Composable
private fun StartChatScreen(onStart: (String) -> Unit, onClose: () -> Unit) {
    var peer by rememberSaveable { mutableStateOf("") }
    val userId = peer.trim()
    WontonScreen(bar = { WontonPushedBar("发起私聊", onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonTextField(
                value = peer,
                onValueChange = { peer = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = "@hermes:wonton.localhost",
                label = "对方的 Matrix ID",
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
            )
            WontonButton("发起", { onStart(userId) }, Modifier.fillMaxWidth(), enabled = userId.isNotEmpty())
        }
    }
}
