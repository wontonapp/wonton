package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.clickable
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.FamilyAction
import app.wonton.protocol.FamilyViewModel
import app.wonton.protocol.JoinRequestRow
import app.wonton.protocol.MemberRow
import app.wonton.protocol.MemberStatus
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.toColor
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

@Composable
fun MembersPage(core: ClientCore, onBack: () -> Unit, onRoles: () -> Unit) {
    val view by remember(core) { core.familyUpdates() }.collectAsState(remember(core) { core.family() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var actionMember by remember { mutableStateOf<MemberRow?>(null) }
    var removing by remember { mutableStateOf<MemberRow?>(null) }
    var assigning by remember { mutableStateOf<MemberRow?>(null) }
    var inviting by remember { mutableStateOf(false) }
    val send: (Action) -> Problem? = { action ->
        core.send(action).also { rejection = it }
    }
    LaunchedEffect(core) { send(Action.Family(FamilyAction.RefreshFamily)) }
    DisposableEffect(core) {
        onDispose { core.send(Action.Family(FamilyAction.LeaveFamily)) }
    }
    BackHandler(onBack = onBack)
    MembersContent(
        view = view,
        onBack = onBack,
        onRoles = onRoles,
        onInvite = { inviting = true },
        onRefresh = { send(Action.Family(FamilyAction.RefreshFamily)) },
        onApprove = { id -> send(Action.Family(FamilyAction.ApproveJoinRequest(id))) },
        onReject = { id -> send(Action.Family(FamilyAction.RejectJoinRequest(id))) },
        onMember = { actionMember = it },
    )
    if (inviting) {
        MemberInviteSheet(core, view, onDismiss = { inviting = false }, onRejection = { rejection = it }) {
            send(Action.Family(FamilyAction.DismissMemberInvite))
        }
    }
    actionMember?.let { member ->
        WontonActionSheet(
            actions = memberActions(view, member, send, onRemove = { removing = it }, onAssign = { assigning = it }),
            title = member.displayName,
            onDismiss = { actionMember = null },
        )
    }
    assigning?.let { member ->
        MemberRolePicker(core, view, member, onDismiss = { assigning = null })
    }
    removing?.let { member ->
        MemberRemoveSheet(core, member, onDismiss = { removing = null })
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

@Composable
private fun Identity(view: FamilyViewModel) {
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(
            stringResource(if (view.access.isOwner) R.string.family_youAreOwner else R.string.family_youAreManager),
            style = WontonTheme.typography.supporting,
            color = secondary,
        )
        if (!view.access.isOwner) {
            view.access.roleName?.let { Text(it, style = WontonTheme.typography.body) }
        }
    }
}


/// 成员页的静态内容：ViewModel 和回调都由页面给，预览和截图直接喂假数据。
@Composable
internal fun MembersContent(
    view: FamilyViewModel,
    onBack: () -> Unit,
    onRoles: () -> Unit,
    onInvite: () -> Unit,
    onRefresh: () -> Unit,
    onApprove: (String) -> Unit,
    onReject: (String) -> Unit,
    onMember: (MemberRow) -> Unit,
) {
    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.family_membersTitle), onBack, actions = {
            if (view.access.canManageRoles()) {
                WontonIconButton(WontonIcon.Person, stringResource(R.string.family_rolesTitle), onRoles)
            }
            if (view.access.canManageMembers()) {
                WontonIconButton(WontonIcon.Add, stringResource(R.string.family_invite), onInvite)
            }
        })
    }) {
        Column(
            Modifier.fillMaxSize().wontonContentPadding().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            Identity(view)
            view.members.problem?.let { problem ->
                ProblemRow(problem.problem, onRefresh)
            }
            if (view.access.canManageMembers()) {
                Requests(view, onRefresh = onRefresh, onApprove = onApprove, onReject = onReject)
            }
            Members(view, onAction = onMember)
        }
    }
}

@Composable
internal fun Requests(
    view: FamilyViewModel,
    onRefresh: () -> Unit,
    onApprove: (String) -> Unit,
    onReject: (String) -> Unit,
) {
    if (view.joinRequests.requests.isEmpty() && view.joinRequests.problem == null) return
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        SectionHint(stringResource(R.string.family_requestsSection))
        view.joinRequests.problem?.let { problem ->
            ProblemRow(problem.problem, onRefresh)
        }
        view.joinRequests.requests.forEach { request ->
            JoinRequestCard(
                request,
                approve = { onApprove(request.id) },
                reject = { onReject(request.id) },
            )
        }
    }
}
@Composable
internal fun Members(view: FamilyViewModel, onAction: (MemberRow) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        SectionHint(stringResource(R.string.family_membersSection))
        if (view.members.members.isEmpty()) {
            if (view.members.loading) {
                CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall))
            } else {
                Text(
                    stringResource(R.string.family_empty),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        } else {
            WontonGroup {
                view.members.members.forEachIndexed { index, member ->
                    if (index > 0) WontonSeparator()
                    MemberRowView(member) { onAction(member) }
                }
            }
        }
    }
}

@Composable
internal fun SectionHint(text: String) {
    Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
}

@Composable
private fun MemberRowView(member: MemberRow, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick)
            .padding(WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
            MemberInitial(member.displayName)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    Text(member.displayName, style = WontonTheme.typography.body)
                    Text(
                        roleName(member.roleName, member.roleTemplate, member.roleId),
                        style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.textSecondary.toColor(),
                    )
                }
                Text(member.username, style = WontonTheme.typography.caption,
                    color = WontonTheme.colors.textSecondary.toColor())
                when (member.status) {
                    MemberStatus.INVITED -> member.code?.let {
                        Text(codeRemaining(it), style = WontonTheme.typography.caption,
                            color = if (it.expired) WontonTheme.colors.danger.toColor()
                            else WontonTheme.colors.textSecondary.toColor())
                    } ?: Text(stringResource(R.string.family_statusInvited),
                        style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.textSecondary.toColor())
                    else -> Text(memberStatus(member.status), style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.textSecondary.toColor())
                }
            }
        }
}

@Composable
private fun MemberInitial(name: String) {
    Box(
        Modifier.size(WontonTheme.size.target).clip(CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            name.take(1).uppercase().ifEmpty { "?" },
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun JoinRequestCard(request: JoinRequestRow, approve: () -> Unit, reject: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            MemberInitial(request.displayName)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    Text(request.displayName, style = WontonTheme.typography.body)
                    Text(
                        roleName(request.roleName, request.roleTemplate, request.roleId),
                        style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.textSecondary.toColor(),
                    )
                }
                Text(request.deviceName, style = WontonTheme.typography.caption,
                    color = WontonTheme.colors.textSecondary.toColor())
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(
                stringResource(R.string.family_approve),
                approve,
                Modifier.weight(1f),
                kind = WontonButtonKind.PRIMARY,
            )
            WontonButton(
                stringResource(R.string.family_reject),
                reject,
                Modifier.weight(1f),
                kind = WontonButtonKind.SECONDARY,
            )
        }
    }
}

@Composable
private fun memberActions(
    view: FamilyViewModel,
    member: MemberRow,
    send: (Action) -> Problem?,
    onRemove: (MemberRow) -> Unit,
    onAssign: (MemberRow) -> Unit,
): List<WontonSheetAction> {
    val actions = mutableListOf<WontonSheetAction>()
    if (member.status == MemberStatus.INVITED) {
        if (member.code != null) {
            actions += WontonSheetAction(
                stringResource(R.string.family_resendCode),
                icon = WontonIcon.Refresh,
                onClick = { send(Action.Family(FamilyAction.ResendMemberCode(member.username))) },
            )
            actions += WontonSheetAction(
                stringResource(R.string.family_revokeCode),
                icon = WontonIcon.Trash,
                onClick = { send(Action.Family(FamilyAction.RevokeMemberCode(member.username))) },
            )
        } else {
            actions += WontonSheetAction(
                stringResource(R.string.family_sendCode),
                icon = WontonIcon.Refresh,
                onClick = { send(Action.Family(FamilyAction.ResendMemberCode(member.username))) },
            )
        }
    }
    if (view.access.canManageRoles()) {
        actions += WontonSheetAction(
            stringResource(R.string.family_changeRole),
            icon = WontonIcon.Person,
            onClick = { onAssign(member) },
        )
    }
    actions += WontonSheetAction(
        stringResource(R.string.family_removeMember),
        icon = WontonIcon.Trash,
        isDanger = true,
        onClick = { onRemove(member) },
    )
    return actions
}
