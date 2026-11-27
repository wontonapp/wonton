package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AppLink
import app.wonton.protocol.BackupReminder
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.ConfirmationsViewModel
import kotlinx.coroutines.CoroutineScope

@Suppress("LongParameterList") // 聊天这一层的每一页各要一部分，拆开要再包一层状态。
@Composable
internal fun ChatPageContent(
    page: ChatPage,
    core: ClientCore,
    navigation: ChatNavigation,
    viewModel: ChatViewModel,
    confirmations: ConfirmationsViewModel,
    plugins: PluginsState,
    pluginNavigation: PluginNavigation,
    onAction: (Action) -> Unit,
    links: ChatLinks,
    onOpenApps: () -> Unit,
    onReject: (Problem?) -> Unit,
) {
    when (page) {
        ChatPage.Confirmations -> ConfirmationsScreen(confirmations, viewModel.bots, plugins, onAction) {
            navigation.showingConfirmations = false
        }
        ChatPage.Backup -> ChatBackupScreen(
            viewModel.encryption,
            onAction,
            onContinueHandoff = { navigation.keyHandoffDeferred = false },
            onClose = { navigation.showingBackup = false },
        )
        is ChatPage.Profile -> BotProfileScreen(
            page.bot, viewModel.bots.firstOrNull { it.bot == page.bot }, plugins, onAction,
            onClose = { navigation.botProfile = null },
        )
        ChatPage.Starting -> StartChatScreen(
            core = core,
            viewModel = viewModel,
            plugins = plugins,
            onStart = { userId, contact ->
                navigation.startingChat = false
                onReject(if (contact != null) {
                    navigation.startChatWith(core, contact) { row -> viewModel.bots.directBot(row) != null }
                } else {
                    navigation.openDirectChat(core, userId)
                })
            },
            onClose = { navigation.startingChat = false },
        )
        is ChatPage.Room -> ChatRoomPage(core, navigation, page.id, viewModel, plugins, onAction, links, onReject)
        ChatPage.RoomList -> if (LocalExpandedChat.current) {
            PluginPages(plugins, pluginNavigation) { ChatDetailEmpty() }
        } else PluginPages(plugins, pluginNavigation) {
            ChatListScreen(
                viewModel = viewModel,
                plugins = plugins,
                pluginNavigation = pluginNavigation,
                confirmationCount = confirmations.pending.size + confirmations.contactRequests.size,
                onOpenConfirmations = navigation::openConfirmations,
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
}

/** 房间里的链接：协程跟着整个聊天层（关房间不打断正在打开的链接），应用内链接交给上层。 */
internal data class ChatLinks(val scope: CoroutineScope, val onAppLink: (AppLink) -> Unit)

@Suppress("LongParameterList")
@Composable
private fun ChatRoomPage(
    core: ClientCore,
    navigation: ChatNavigation,
    room: String,
    viewModel: ChatViewModel,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    links: ChatLinks,
    onReject: (Problem?) -> Unit,
) {
    // 关房间要回别的页时（[ChatNavigation.returnsElsewhere]）下面露出来的不是聊天列表，不预览，直接关。
    BackHandler(enabled = navigation.returnsElsewhere) { navigation.closeRoom() }
    val row = viewModel.rooms.firstOrNull { it.roomId == room }
    val context = LocalContext.current
    val opener = remember(core, navigation, context, links) {
        AndroidLinkOpener(context, core, navigation, links.scope, links.onAppLink) { onReject(it) }
    }
    CompositionLocalProvider(LocalLinkOpener provides opener) {
        OpenRoom(core, navigation, room, row, viewModel, plugins, onAction) { onReject(it) }
    }
}
