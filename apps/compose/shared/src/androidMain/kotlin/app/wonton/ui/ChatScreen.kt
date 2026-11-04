package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatSync
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.RecoveryStatus
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow

@Composable
fun ChatRoute(core: ClientCore, navigation: ChatNavigation) {
    val updates = remember(core) { core.chatUpdates() }
    val initial = remember(core) { core.chat() }
    val viewModel by updates.collectAsState(initial = initial)
    var rejection by remember { mutableStateOf<String?>(null) }
    var showingEncryption by rememberSaveable { mutableStateOf(false) }
    // 只在被拒绝时更新提示：成功的操作（比如自动发出的已读回执）不清掉还没看的拒绝原因。
    val onAction: (Action) -> Unit = { action -> core.send(action)?.let { rejection = it } }

    LaunchedEffect(viewModel) { navigation.onUpdate(viewModel) }

    val room = navigation.openRoom
    // 提示条和通知能在加密页面上打开房间。
    LaunchedEffect(room) { if (room != null) showingEncryption = false }
    when {
        showingEncryption -> {
            BackHandler { showingEncryption = false }
            EncryptionScreen(viewModel.encryption, onAction, onClose = { showingEncryption = false })
        }
        room != null -> {
            BackHandler { navigation.openRoom = null }
            val row = viewModel.rooms.firstOrNull { it.roomId == room }
            RoomRoute(
                core = core,
                roomId = room,
                title = row?.name ?: room,
                isDirect = row?.isDirect == true,
                onAction = onAction,
                onReject = { rejection = it },
                onClose = { navigation.openRoom = null },
            )
        }
        else -> ChatListScreen(
            viewModel = viewModel,
            onAction = onAction,
            onOpenRoom = { navigation.openRoom = it },
            onOpenEncryption = { showingEncryption = true },
        )
    }

    viewModel.problem?.let { problem ->
        AlertDialog(
            onDismissRequest = { onAction(Action.DismissChatProblem) },
            confirmButton = { TextButton(onClick = { onAction(Action.DismissChatProblem) }) { Text("好") } },
            title = { Text("操作失败") },
            text = { Text(problem.message) },
        )
    }
    rejection?.let { reason ->
        AlertDialog(
            onDismissRequest = { rejection = null },
            confirmButton = { TextButton(onClick = { rejection = null }) { Text("好") } },
            title = { Text("操作被拒绝") },
            text = { Text(reason) },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChatListScreen(
    viewModel: ChatViewModel,
    onAction: (Action) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenEncryption: () -> Unit,
) {
    var startingChat by rememberSaveable { mutableStateOf(false) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("聊天") },
                actions = {
                    TextButton(onClick = onOpenEncryption) { Text("加密") }
                    TextButton(onClick = { startingChat = true }) { Text("发起私聊") }
                },
            )
        },
    ) { padding ->
        val status = statusText(viewModel)
        val reminder = encryptionReminder(viewModel)
        LazyColumn(Modifier.fillMaxSize(), contentPadding = padding) {
            status?.let { text ->
                item(key = "status") {
                    Text(
                        text,
                        Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
            reminder?.let { text ->
                item(key = "reminder") {
                    TextButton(onClick = onOpenEncryption, Modifier.padding(horizontal = 8.dp)) { Text(text) }
                }
            }
            items(viewModel.rooms, key = { it.roomId }) { row ->
                if (row.membership == RoomMembership.INVITED) {
                    InviteItem(row, onAction)
                } else {
                    RoomItem(row, onClick = { onOpenRoom(row.roomId) })
                }
            }
        }
        if (viewModel.rooms.isEmpty() && viewModel.account is ChatAccount.SignedIn) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                Text("还没有聊天")
            }
        }
    }
    if (startingChat) {
        StartChatDialog(
            onStart = { userId ->
                startingChat = false
                onAction(Action.StartDirectChat(userId))
            },
            onDismiss = { startingChat = false },
        )
    }
}

private fun statusText(viewModel: ChatViewModel): String? =
    when (val account = viewModel.account) {
        ChatAccount.SignedOut -> "还没有连上家里的服务器"
        ChatAccount.SigningIn -> "正在登录…"
        is ChatAccount.Failed -> account.message
        is ChatAccount.SignedIn -> when (val sync = viewModel.sync) {
            ChatSync.Idle, ChatSync.Starting -> "正在同步…"
            ChatSync.Running -> null
            is ChatSync.Failed -> sync.message
        }
    }

private fun encryptionReminder(viewModel: ChatViewModel): String? =
    when {
        viewModel.encryption.recoveryKey != null -> "恢复密钥还没确认保存"
        viewModel.encryption.recovery == RecoveryStatus.INCOMPLETE -> "输入恢复密钥，才能解密以前的加密消息"
        else -> null
    }

@Composable
private fun RoomItem(row: RoomRow, onClick: () -> Unit) {
    ListItem(
        headlineContent = {
            Text(if (row.isEncrypted) "${row.name} 🔒" else row.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = row.preview?.let { preview ->
            {
                Text(
                    if (preview.isOwn) "我：${preview.text}" else preview.text,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        },
        trailingContent = if (row.unreadCount > 0uL) {
            {
                Text(
                    row.unreadCount.toString(),
                    Modifier.background(MaterialTheme.colorScheme.error, CircleShape)
                        .padding(horizontal = 8.dp, vertical = 2.dp),
                    color = MaterialTheme.colorScheme.onError,
                    style = MaterialTheme.typography.labelSmall,
                )
            }
        } else {
            null
        },
        modifier = Modifier.clickable(onClick = onClick),
    )
}

@Composable
private fun InviteItem(row: RoomRow, onAction: (Action) -> Unit) {
    ListItem(
        headlineContent = { Text("${row.name} 邀请你${if (row.isDirect) "私聊" else "加入房间"}") },
        supportingContent = {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { onAction(Action.JoinRoom(row.roomId)) }) { Text("接受") }
                OutlinedButton(onClick = { onAction(Action.LeaveRoom(row.roomId)) }) { Text("拒绝") }
            }
        },
    )
}

@Composable
private fun StartChatDialog(onStart: (String) -> Unit, onDismiss: () -> Unit) {
    var userId by rememberSaveable { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("发起私聊") },
        text = {
            OutlinedTextField(
                value = userId,
                onValueChange = { userId = it },
                placeholder = { Text("@hermes:wonton.localhost") },
                singleLine = true,
            )
        },
        confirmButton = { TextButton(onClick = { onStart(userId) }) { Text("发起") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
    )
}
