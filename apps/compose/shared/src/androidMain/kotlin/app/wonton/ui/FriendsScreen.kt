package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.FriendsAction
import app.wonton.protocol.FriendBlockScope
import app.wonton.protocol.FriendBlockTarget
import app.wonton.protocol.FriendGroup
import app.wonton.protocol.FriendRow
import app.wonton.protocol.FriendsState
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
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
    var scanning by remember { mutableStateOf(false) }
    var showingMyCode by remember { mutableStateOf(false) }
    var menus by remember { mutableStateOf<FriendRow?>(null) }
    var choosingGroups by remember { mutableStateOf<FriendRow?>(null) }
    var removing by remember { mutableStateOf<FriendRow?>(null) }
    var creatingGroup by remember { mutableStateOf(false) }
    var renamingGroup by remember { mutableStateOf<FriendGroup?>(null) }
    var removingGroup by remember { mutableStateOf<FriendGroup?>(null) }
    var blockingServer by remember { mutableStateOf(false) }
    val send = { action: Action -> rejection = core.send(action) }
    LaunchedEffect(core) { send(Action.Friends(FriendsAction.RefreshFriends)) }
    if (scanning) {
        FriendScanPage(
            onBack = { scanning = false },
            // 扫到什么就交什么，解析在核心里。
            onSubmit = {
                scanning = false
                send(Action.Friends(FriendsAction.AddFriend(it)))
            },
        )
    } else {
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
        onScan = { scanning = true },
        onShowMyCode = { showingMyCode = true },
        setMenus = { menus = it },
        setChoosingGroups = { choosingGroups = it },
        setRemoving = { removing = it },
        setCreatingGroup = { creatingGroup = it },
        setRenamingGroup = { renamingGroup = it },
        setRemovingGroup = { removingGroup = it },
        setBlockingServer = { blockingServer = it },
    )
    if (showingMyCode) {
        friends.myLink?.let { link -> MyFriendCodeDialog(link) { showingMyCode = false } }
    }
}

/** 扫对方的码：取景框扫，扫不到就粘贴；原文原样交给核心，是 ID 还是链接由核心认。 */
@Composable
private fun FriendScanPage(onBack: () -> Unit, onSubmit: (String) -> Unit) {
    var pasted by rememberSaveable { mutableStateOf("") }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.friends_scan_code), onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            CameraArea(onCode = onSubmit)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonTextField(
                    pasted,
                    { pasted = it },
                    Modifier.fillMaxWidth(),
                    placeholder = stringResource(R.string.friends_paste_placeholder),
                    maxLines = PASTED_CODE_LINES,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
                WontonButton(
                    stringResource(R.string.friends_add_submit),
                    { onSubmit(pasted) },
                    Modifier.fillMaxWidth(),
                    enabled = pasted.isNotBlank(),
                )
            }
        }
    }
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
            onRetry = { send(Action.Friends(FriendsAction.RefreshFriends)) },
            onAction = send,
            onAdd = onAdd,
            onMenu = onMenu,
            onNewGroup = onNewGroup,
            onRenameGroup = onRenameGroup,
            onDeleteGroup = onDeleteGroup,
            onBlockServer = onBlockServer,
        )
        state is FriendsState.Failed -> ProblemView(state.problem()) {
            send(Action.Friends(FriendsAction.RefreshFriends))
        }
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
    onScan: () -> Unit,
    onShowMyCode: () -> Unit,
    setMenus: (FriendRow?) -> Unit,
    setChoosingGroups: (FriendRow?) -> Unit,
    setRemoving: (FriendRow?) -> Unit,
    setCreatingGroup: (Boolean) -> Unit,
    setRenamingGroup: (FriendGroup?) -> Unit,
    setRemovingGroup: (FriendGroup?) -> Unit,
    setBlockingServer: (Boolean) -> Unit,
) {
    if (adding) {
        AddFriendSheet(
            friends.myLink,
            onScan,
            onShowMyCode,
            { send(Action.Friends(FriendsAction.AddFriend(it))) },
            { setAdding(false) },
        )
    }
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
                    send(
                        Action.Friends(
                            FriendsAction.BlockFriend(
                                FriendBlockTarget.User(row.userId),
                                FriendBlockScope.INVITATIONS,
                            ),
                        ),
                    )
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
        ChooseGroupsSheet(
            row,
            friends.groups,
            { send(Action.Friends(FriendsAction.SetFriendGroups(row.userId, it.toList()))) },
        ) {
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
                    send(Action.Friends(FriendsAction.RemoveFriend(row.userId)))
                },
            ),
        )
    }
    if (creatingGroup) {
        GroupNameDialog(stringResource(R.string.friends_group_create_title), "", { setCreatingGroup(false) }) {
            setCreatingGroup(false)
            send(Action.Friends(FriendsAction.SaveFriendGroup(null, it)))
        }
    }
    renamingGroup?.let { group ->
        GroupNameDialog(group.name, group.name, { setRenamingGroup(null) }) {
            setRenamingGroup(null)
            send(Action.Friends(FriendsAction.SaveFriendGroup(group.id, it)))
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
                    send(Action.Friends(FriendsAction.RemoveFriendGroup(group.id)))
                },
            ),
        )
    }
    if (blockingServer) {
        BlockServerDialog({ setBlockingServer(false) }) {
            setBlockingServer(false)
            send(Action.Friends(FriendsAction.BlockFriend(FriendBlockTarget.Server(it), FriendBlockScope.FEDERATION)))
        }
    }
    rejection?.let { ProblemDialog(it, clearRejection) }
}
