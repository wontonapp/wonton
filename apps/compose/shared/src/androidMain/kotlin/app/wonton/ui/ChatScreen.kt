package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.ChatSync
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.BotView
import app.wonton.protocol.BackupReminder
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Connection
import app.wonton.ui.icons.LockFill
import app.wonton.ui.icons.WontonIcon
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

private const val DAYS_SHOWN_AS_WEEKDAY = 7L

@Composable
fun ChatRoute(
    core: ClientCore,
    navigation: ChatNavigation,
    plugins: PluginsState,
    pluginNavigation: PluginNavigation,
    onOpenApps: () -> Unit,
) {
    val updates = remember(core) { core.chatUpdates() }
    val initial = remember(core) { core.chat() }
    val viewModel by updates.collectAsState(initial = initial)
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val scope = rememberCoroutineScope()
    // 只在被拒绝时更新提示：成功的操作（比如自动发出的已读回执）不清掉还没看的拒绝原因。
    val onAction: (Action) -> Unit = { action -> core.send(action)?.let { rejection = it } }

    val room = navigation.openRoom
    // 提示条和通知能在备份页面上打开房间。
    LaunchedEffect(room) { if (room != null) navigation.showingBackup = false }
    when {
        navigation.showingBackup -> {
            BackHandler { navigation.showingBackup = false }
            ChatBackupScreen(
                viewModel.encryption,
                onAction,
                onContinueHandoff = { navigation.keyHandoffDeferred = false },
                onClose = { navigation.showingBackup = false },
            )
        }
        navigation.startingChat -> {
            BackHandler { navigation.startingChat = false }
            StartChatScreen(
                core = core,
                onStart = { userId ->
                    navigation.startingChat = false
                    rejection = navigation.openDirectChat(core, userId)
                },
                onClose = { navigation.startingChat = false },
            )
        }
        room != null -> {
            // 面板开着时返回键先关面板，不关房间。
            val panel = LocalChatPanel.current
            BackHandler {
                if (panel != null && panel.level != PanelLevel.CLOSED) panel.back() else navigation.closeRoom()
            }
            val row = viewModel.rooms.firstOrNull { it.roomId == room }
            val context = LocalContext.current
            val opener = remember(core, navigation, context, scope) {
                AndroidLinkOpener(context, core, navigation, scope) { rejection = it }
            }
            CompositionLocalProvider(LocalLinkOpener provides opener) {
                RoomRoute(
                    core = core,
                    roomId = room,
                    title = row?.name ?: room,
                    isDirect = row?.isDirect == true,
                    isReadOnly = row?.isReadOnly == true,
                    bot = viewModel.bots.directBot(row),
                    plugins = plugins,
                    rooms = viewModel.rooms,
                    onAction = onAction,
                    onReject = { rejection = it },
                    onOpenRoom = { navigation.openRoom = it },
                    onOpenBackup = { navigation.showingBackup = true },
                    onClose = { if (navigation.openRoom == room) navigation.closeRoom() },
                )
            }
        }
        else -> PluginPages(plugins, pluginNavigation) {
            ChatListScreen(
                viewModel = viewModel,
                plugins = plugins,
                pluginNavigation = pluginNavigation,
                onAction = onAction,
                onOpenRoom = { navigation.openRoom = it },
                onOpenBackup = {
                    // 收起了的交接页从提醒再打开，不进备份页（ADR-0087）。
                    if (viewModel.encryption.backupReminder == BackupReminder.CONTINUE_ON_OTHER_DEVICE) {
                        navigation.keyHandoffDeferred = false
                    } else navigation.showingBackup = true
                },
                onStartChat = { navigation.startingChat = true },
                onOpenApps = onOpenApps,
            )
        }
    }

    viewModel.problem?.let { ProblemDialog(it.problem) { onAction(Action.DismissChatProblem) } }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

@Composable
private fun ChatListScreen(
    viewModel: ChatViewModel,
    plugins: PluginsState,
    pluginNavigation: PluginNavigation,
    onAction: (Action) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenBackup: () -> Unit,
    onStartChat: () -> Unit,
    onOpenApps: () -> Unit,
) {
    val installing = viewModel.installingBots
    val panel = LocalChatPanel.current
    val listState = rememberLazyListState()
    WontonScreen(
        bar = { WontonRootBar(stringResource(R.string.tab_chat)) },
    ) {
        Box(Modifier.fillMaxSize()) {
            LazyColumn(
                Modifier.fillMaxSize()
                    .panelAction(panel)
                    .chatPanelPull(panel, { !listState.canScrollBackward }, panel != null),
                state = listState,
                overscrollEffect = null,
                contentPadding = wontonContentPaddingValues(
                    bottom = WontonTheme.size.floatingActionButton + WontonTheme.spacing.pageInset * 2,
                ),
            ) {
                item(key = "notices") { Notices(viewModel, onOpenBackup) }
                itemsIndexed(installing, key = { _, row -> "install:${row.pluginId}" }) { index, row ->
                    if (index > 0) {
                        Box(Modifier.padding(start = WontonTheme.size.avatar + WontonTheme.spacing.s12)) {
                            WontonSeparator()
                        }
                    }
                    ChatInstallRow(
                        row,
                        plugins,
                        onRetry = {
                            // 重试不进核心的执行路径：重新预览，主人在确认页上再点一次（ADR-0067）。
                            pluginNavigation.confirmation = when (row.operation) {
                                PluginOperation.UPGRADE -> PluginConfirmation.Upgrade(row.pluginId)
                                else -> row.source?.let { PluginConfirmation.Install(it) }
                            }
                        },
                        onOpenPage = onOpenApps,
                    )
                }
                itemsIndexed(viewModel.rooms, key = { _, row -> row.roomId }) { index, row ->
                    if (index > 0 || installing.isNotEmpty()) {
                        Box(Modifier.padding(start = WontonTheme.size.avatar + WontonTheme.spacing.s12)) {
                            WontonSeparator()
                        }
                    }
                    if (row.membership == RoomMembership.INVITED) {
                        InviteItem(row, onAction)
                    } else {
                        RoomItem(row, viewModel.bots.directBot(row), plugins, onClick = { onOpenRoom(row.roomId) })
                    }
                }
            }
            // 同步好之前列表是空的不代表没有聊天。
            val synced = viewModel.account is ChatAccount.SignedIn && viewModel.sync == ChatSync.Running
            if (viewModel.rooms.isEmpty() && installing.isEmpty() && synced) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        stringResource(R.string.chat_empty),
                        style = WontonTheme.typography.itemTitle,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                }
            }
            FloatingActionButton(
                onClick = onStartChat,
                modifier = Modifier.align(Alignment.BottomEnd)
                    .padding(
                        end = WontonTheme.spacing.pageInset,
                        bottom = LocalWontonBottomInset.current + WontonTheme.spacing.pageInset,
                    )
                    .size(WontonTheme.size.floatingActionButton),
                containerColor = WontonTheme.colors.accentFill.toColor(),
                contentColor = WontonTheme.colors.accentInkOnFill.toColor(),
            ) {
                Icon(WontonIcon.Add, stringResource(R.string.shortcut_new_chat), Modifier.size(WontonTheme.size.icon))
            }
        }
    }
}

@Composable
internal fun Notices(viewModel: ChatViewModel, onOpenBackup: () -> Unit) {
    val account = viewModel.account
    val sync = viewModel.sync
    val backupReminder = viewModel.encryption.backupReminder
    val reminder = backupReminder?.text
    // 连不上由标题栏说，连上后自动重来。
    val failure = (account as? ChatAccount.Failed)?.takeIf { it.reason != ChatFailure.TRANSPORT_UNAVAILABLE }
        ?.let { it.reason.problem(it.message) }
        ?: (sync as? ChatSync.Failed)?.takeIf { it.reason != ChatFailure.TRANSPORT_UNAVAILABLE }
            ?.let { it.reason.problem(it.message) }
            .takeIf { account is ChatAccount.SignedIn }
    val progress = when (account) {
        ChatAccount.SigningIn -> stringResource(R.string.chat_signing_in)
        is ChatAccount.Reconnecting -> stringResource(R.string.chat_status_reconnecting)
        else -> null
    }
    val context = LocalContext.current
    val reminderPrefs = remember { BackupReminderPrefs(context) }
    var reminderDismissed by remember(backupReminder) {
        mutableStateOf(backupReminder != null && reminderPrefs.dismissed(backupReminder))
    }
    if (account != ChatAccount.SignedOut && listOfNotNull(failure, progress, reminder).isEmpty()) return
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        if (account == ChatAccount.SignedOut) {
            WontonNotice(WontonIcon.Connection, stringResource(R.string.chat_signed_out), isError = false)
        }
        failure?.let { ProblemRow(it) }
        progress?.let {
            Text(it, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
        }
        if (reminder != null && !reminderDismissed && backupReminder != null) {
            WontonGroup {
                Row(
                    Modifier.fillMaxWidth()
                        .clickable(onClick = onOpenBackup)
                        .heightIn(min = WontonTheme.size.target)
                        .padding(start = WontonTheme.spacing.s16),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        reminder,
                        Modifier.weight(1f).padding(vertical = WontonTheme.spacing.s12),
                        style = WontonTheme.typography.body,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                    WontonIconButton(
                        WontonIcon.Close,
                        stringResource(R.string.common_close),
                        {
                            reminderPrefs.dismiss(backupReminder)
                            reminderDismissed = true
                        },
                    )
                }
            }
        }
    }
}

/** 和 Bot 的私聊行对应的 Bot；不是私聊、或没有 Bot 的房间为空。 */
private fun List<BotView>.directBot(row: RoomRow?): BotView? =
    row?.directUserId?.let { direct -> firstOrNull { it.userId == direct } }

@Composable
private fun RoomItem(row: RoomRow, bot: BotView?, plugins: PluginsState, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        RoomAvatar(row, bot, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Row(
                    Modifier.weight(1f),
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
                            contentDescription = stringResource(R.string.chat_encrypted),
                            modifier = Modifier.size(WontonTheme.size.iconTiny),
                            tint = colors.textSecondary.toColor(),
                        )
                    }
                }
                row.preview?.let {
                    Text(
                        roomTime(it.timestampMs.toLong()),
                        style = WontonTheme.typography.caption.copy(fontFeatureSettings = "tnum"),
                        color = colors.textSecondary.toColor(),
                    )
                }
            }
            if (row.preview != null || row.unreadCount > 0uL) RoomPreviewLine(row)
        }
    }
}

@Composable
private fun RoomPreviewLine(row: RoomRow) {
    val colors = WontonTheme.colors
    val resources = LocalContext.current.resources
    val preview = row.preview?.body?.line(resources).orEmpty()
    val text = if (row.preview?.isOwn == true) stringResource(R.string.chat_own_preview, preview) else preview
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(
            text,
            Modifier.weight(1f),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (row.unreadCount > 0uL) {
            // unreadCount 是 ULong，Android 的 %d 格式化不认，转成 Int。
            val spoken = stringResource(R.string.chat_unread_count, row.unreadCount.toInt())
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
private fun RoomAvatar(row: RoomRow, bot: BotView?, plugins: PluginsState) {
    val size = WontonTheme.size.avatar
    if (bot != null) {
        // 和 Bot 的私聊行：显示 Bot 的资料头像（状态头像在播，没有就静态头像、应用图标）。
        BotAvatar(bot, plugins, size)
        return
    }
    val plugin = plugins.viewModel.installed?.plugins?.firstOrNull {
        row.directUserId != null && it.botUserId == row.directUserId
    }
    if (plugin != null) {
        PluginIconImage(plugin.icon, size, plugins)
    } else {
        Box(
            Modifier.size(size).background(WontonTheme.colors.surfaceSubtle.toColor(), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                row.name.take(1).uppercase(),
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
        }
    }
}

/** 列表里的时间：今天的写几点，昨天写“昨天”，一周内写星期，再早写日期。 */
@Composable
private fun roomTime(timestampMs: Long): String {
    val time = Instant.ofEpochMilli(timestampMs).atZone(ZoneId.systemDefault())
    val today = LocalDate.now()
    val days = ChronoUnit.DAYS.between(time.toLocalDate(), today)
    val pattern = when {
        days == 0L -> "HH:mm"
        days == 1L -> return stringResource(R.string.chat_yesterday)
        days < DAYS_SHOWN_AS_WEEKDAY -> "EEE"
        time.year == today.year -> "M/d"
        else -> "yyyy/M/d"
    }
    return time.format(DateTimeFormatter.ofPattern(pattern))
}

@Composable
internal fun InviteItem(row: RoomRow, onAction: (Action) -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            stringResource(if (row.isDirect) R.string.chat_invite_direct else R.string.chat_invite_room, row.name),
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(stringResource(R.string.chat_invite_accept), { onAction(Action.JoinRoom(row.roomId)) })
            WontonButton(
                stringResource(R.string.chat_invite_decline), { onAction(Action.LeaveRoom(row.roomId)) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
    }
}

/** 选一个人或输入用户名发起私聊，房间出现后打开。 */
@Composable
private fun StartChatScreen(core: ClientCore, onStart: (String) -> Unit, onClose: () -> Unit) {
    var peer by rememberSaveable { mutableStateOf("") }
    val input = peer.trim()
    val contacts = remember(input) { core.chatContacts(input) }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.chat_start_direct), onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonTextField(
                value = peer,
                onValueChange = { peer = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.chat_peer_placeholder),
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
            )
            WontonButton(
                stringResource(R.string.chat_start), { onStart(input) }, Modifier.fillMaxWidth(),
                enabled = input.isNotEmpty(),
            )
            if (contacts.isNotEmpty()) {
                WontonGroup {
                    contacts.forEachIndexed { index, contact ->
                        if (index > 0) WontonSeparator()
                        WontonRow(contact.name, showsForward = true, onClick = { onStart(contact.userId) })
                    }
                }
            }
        }
    }
}
