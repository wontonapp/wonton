package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.clearAndSetSemantics
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.RoomRow
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 开着的房间：主对话，开着旁聊时旁聊页盖在上面（ADR-0093）。主对话在下面照常订阅着，返回时停在原来的位置；
 * 被盖住时不标已读、屏幕阅读器也读不到它。返回键先关旁聊。
 */
@Composable
internal fun OpenRoom(
    core: ClientCore,
    navigation: ChatNavigation,
    room: String,
    row: RoomRow?,
    viewModel: ChatViewModel,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    onReject: (Problem) -> Unit,
) {
    val sideChat = navigation.sideChat
    val title = row?.name ?: room
    val isDirect = row?.isDirect == true
    val isReadOnly = row?.isReadOnly == true
    val bot = viewModel.bots.directBot(row)
    val main = SideChatEntry(
        supported = row?.sideChats == true && !isReadOnly,
        covered = sideChat != null,
        composing = navigation.composingSideChat,
        unsupportedHint = navigation.noSideChatsRoom == room,
        onOpen = { root -> navigation.openSideChat(room, root) },
        onComposingChange = { navigation.composingSideChat = it },
    )
    Box(Modifier.fillMaxSize()) {
        Box(if (sideChat != null) Modifier.clearAndSetSemantics {} else Modifier) {
            RoomRoute(
                core = core,
                target = TimelineTarget(room, null),
                sideChats = main,
                title = title,
                isDirect = isDirect,
                isReadOnly = isReadOnly,
                bot = bot,
                plugins = plugins,
                rooms = viewModel.rooms,
                onAction = onAction,
                onReject = onReject,
                onOpenRoom = { navigation.openRoom = it },
                onOpenBackup = { navigation.showingBackup = true },
                onClose = { if (navigation.openRoom == room) navigation.closeRoom() },
            )
        }
        if (sideChat != null) {
            BackHandler { navigation.sideChat = null }
            // 盖在主对话上：底色挡住下面的画面，空白处的触摸也不漏到下面。
            Box(
                Modifier.fillMaxSize()
                    .background(WontonTheme.colors.canvas.toColor())
                    .pointerInput(Unit) { awaitPointerEventScope { while (true) awaitPointerEvent() } },
            ) {
                RoomRoute(
                    core = core,
                    target = TimelineTarget(room, sideChat),
                    sideChats = SideChatEntry.NONE,
                    title = title,
                    isDirect = isDirect,
                    isReadOnly = isReadOnly,
                    bot = bot,
                    plugins = plugins,
                    rooms = viewModel.rooms,
                    onAction = onAction,
                    onReject = onReject,
                    onOpenRoom = { navigation.openRoom = it },
                    onOpenBackup = { navigation.showingBackup = true },
                    onClose = { if (navigation.sideChat == sideChat) navigation.sideChat = null },
                )
            }
        }
    }
}
