package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.FriendBlockScope
import app.wonton.protocol.FriendBlockTarget
import app.wonton.protocol.FriendGroup
import app.wonton.protocol.FriendRow
import app.wonton.protocol.FriendsState
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Folder
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun FriendsPage(core: ClientCore, onBack: () -> Unit) {
    val friends by remember(core) { core.friendsUpdates() }.collectAsState(remember(core) { core.friends() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var adding by remember { mutableStateOf(false) }
    var menus by remember { mutableStateOf<FriendRow?>(null) }
    var choosingGroups by remember { mutableStateOf<FriendRow?>(null) }
    var removing by remember { mutableStateOf<FriendRow?>(null) }
    var creatingGroup by remember { mutableStateOf(false) }
    var renamingGroup by remember { mutableStateOf<FriendGroup?>(null) }
    var removingGroup by remember { mutableStateOf<FriendGroup?>(null) }
    var blockingServer by remember { mutableStateOf(false) }
    val send = { action: Action -> rejection = core.send(action) }
    LaunchedEffect(core) { send(Action.RefreshFriends) }
    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.friends_title), onBack, actions = {
            WontonIconButton(WontonIcon.Add, stringResource(R.string.friends_add), { adding = true })
        })
    }) {
        FriendContent(
            friends,
            send,
            { adding = true },
            { menus = it },
            { creatingGroup = true },
            { renamingGroup = it },
            { removingGroup = it },
            { blockingServer = true },
        )
    }
    FriendOverlays(
        friends = friends,
        rejection = rejection,
        adding = adding,
        menus = menus,
        choosingGroups = choosingGroups,
        removing = removing,
        creatingGroup = creatingGroup,
        renamingGroup = renamingGroup,
        removingGroup = removingGroup,
        blockingServer = blockingServer,
        send = send,
        clearRejection = { rejection = null },
        setAdding = { adding = it },
        setMenus = { menus = it },
        setChoosingGroups = { choosingGroups = it },
        setRemoving = { removing = it },
        setCreatingGroup = { creatingGroup = it },
        setRenamingGroup = { renamingGroup = it },
        setRemovingGroup = { removingGroup = it },
        setBlockingServer = { blockingServer = it },
    )
}

@Composable
private fun FriendContent(
    friends: app.wonton.protocol.FriendsViewModel,
    send: (Action) -> Unit,
    onAdd: () -> Unit,
    onMenu: (FriendRow) -> Unit,
    onNewGroup: () -> Unit,
    onRenameGroup: (FriendGroup) -> Unit,
    onDeleteGroup: (FriendGroup) -> Unit,
    onBlockServer: () -> Unit,
) {
    val state = friends.state
    val hasRows = friends.incoming.isNotEmpty() || friends.outgoing.isNotEmpty() || friends.friends.isNotEmpty() ||
        friends.groups.isNotEmpty() || friends.blocks().isNotEmpty()
    when {
        hasRows -> FriendList(
            friends,
            failed = state as? FriendsState.Failed,
            onRetry = { send(Action.RefreshFriends) },
            onAction = send,
            onAdd = onAdd,
            onMenu = onMenu,
            onNewGroup = onNewGroup,
            onRenameGroup = onRenameGroup,
            onDeleteGroup = onDeleteGroup,
            onBlockServer = onBlockServer,
        )
        state is FriendsState.Failed -> ProblemView(state.problem()) { send(Action.RefreshFriends) }
        state == FriendsState.Loading -> LoadingView()
        else -> FriendEmpty(onAdd)
    }
}

@Composable
private fun FriendOverlays(
    friends: app.wonton.protocol.FriendsViewModel,
    rejection: Problem?,
    adding: Boolean,
    menus: FriendRow?,
    choosingGroups: FriendRow?,
    removing: FriendRow?,
    creatingGroup: Boolean,
    renamingGroup: FriendGroup?,
    removingGroup: FriendGroup?,
    blockingServer: Boolean,
    send: (Action) -> Unit,
    clearRejection: () -> Unit,
    setAdding: (Boolean) -> Unit,
    setMenus: (FriendRow?) -> Unit,
    setChoosingGroups: (FriendRow?) -> Unit,
    setRemoving: (FriendRow?) -> Unit,
    setCreatingGroup: (Boolean) -> Unit,
    setRenamingGroup: (FriendGroup?) -> Unit,
    setRemovingGroup: (FriendGroup?) -> Unit,
    setBlockingServer: (Boolean) -> Unit,
) {
    if (adding) AddFriendSheet({ send(Action.AddFriend(it)) }, { setAdding(false) })
    menus?.let { row ->
        WontonActionSheet(
            listOf(
                app.wonton.ui.design.WontonSheetAction(stringResource(R.string.friends_groups), WontonIcon.Folder) {
                    setMenus(null)
                    setChoosingGroups(row)
                },
                app.wonton.ui.design.WontonSheetAction(
                    stringResource(R.string.friends_block_user), WontonIcon.Blocked,
                ) {
                    setMenus(null)
                    send(Action.BlockFriend(FriendBlockTarget.User(row.userId), FriendBlockScope.INVITATIONS))
                },
                app.wonton.ui.design.WontonSheetAction(
                    stringResource(R.string.friends_remove_friend), WontonIcon.Trash, isDanger = true,
                ) {
                    setMenus(null)
                    setRemoving(row)
                },
            ),
            onDismiss = { setMenus(null) },
            title = row.name(),
        )
    }
    choosingGroups?.let { row ->
        ChooseGroupsSheet(row, friends.groups, { send(Action.SetFriendGroups(row.userId, it.toList())) }) {
            setChoosingGroups(null)
        }
    }
    removing?.let { row ->
        WontonDialog(
            stringResource(R.string.friends_remove_title, row.name()),
            stringResource(R.string.friends_remove_message),
            { setRemoving(null) },
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel)) { setRemoving(null) },
                WontonDialogAction(stringResource(R.string.friends_remove_friend), WontonButtonKind.DANGER) {
                    setRemoving(null)
                    send(Action.RemoveFriend(row.userId))
                },
            ),
        )
    }
    if (creatingGroup) {
        GroupNameDialog(stringResource(R.string.friends_group_create_title), "", { setCreatingGroup(false) }) {
            setCreatingGroup(false)
            send(Action.SaveFriendGroup(null, it))
        }
    }
    renamingGroup?.let { group ->
        GroupNameDialog(group.name, group.name, { setRenamingGroup(null) }) {
            setRenamingGroup(null)
            send(Action.SaveFriendGroup(group.id, it))
        }
    }
    removingGroup?.let { group ->
        WontonDialog(
            stringResource(R.string.friends_group_delete_title, group.name),
            stringResource(R.string.friends_group_delete_message),
            { setRemovingGroup(null) },
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel)) { setRemovingGroup(null) },
                WontonDialogAction(stringResource(R.string.common_delete), WontonButtonKind.DANGER) {
                    setRemovingGroup(null)
                    send(Action.RemoveFriendGroup(group.id))
                },
            ),
        )
    }
    if (blockingServer) {
        BlockServerDialog({ setBlockingServer(false) }) {
            setBlockingServer(false)
            send(Action.BlockFriend(FriendBlockTarget.Server(it), FriendBlockScope.FEDERATION))
        }
    }
    rejection?.let { ProblemDialog(it, clearRejection) }
}
