package app.wonton.ui

import android.content.Intent
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.FamilyAction
import app.wonton.protocol.FamilyViewModel
import app.wonton.protocol.MemberRow
import app.wonton.protocol.RoleDef
import app.wonton.protocol.RoleTemplate
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
fun MemberInviteSheet(
    core: ClientCore,
    view: FamilyViewModel,
    onDismiss: () -> Unit,
    onRejection: (Problem) -> Unit,
    onDone: () -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var roleId by remember {
        mutableStateOf(view.roles.defaultInviteRole ?: view.roles.roles.firstOrNull()?.id)
    }
    var privateBot by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val role = view.roles.roles.firstOrNull { it.id == roleId }
    val canSubmit = name.isNotBlank()
    WontonSheet(onDismiss, stringResource(R.string.family_inviteTitle)) { close ->
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            if (view.invite != null) {
                InviteCodePanel(core, view, onDone) { close {} }
            } else {
                WontonTextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = stringResource(R.string.family_displayName),
                )
                WontonTextField(
                    value = username,
                    onValueChange = { username = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = stringResource(R.string.family_username),
                    placeholder = stringResource(R.string.family_usernameHint),
                )
                if (view.access.choosesRole()) {
                    RolePicker(view, role, menu, { menu = it }, { roleId = it })
                }
                if (role?.permissions?.privateBot == true) {
                    Row(
                        Modifier.fillMaxWidth().toggleable(privateBot, role = Role.Switch) { privateBot = it }
                            .padding(WontonTheme.spacing.s12),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                    ) {
                        Text(stringResource(R.string.family_privateBot), Modifier.weight(1f))
                        Switch(privateBot, onCheckedChange = null)
                    }
                }
                view.members.problem?.let { problem ->
                    ProblemRow(problem.problem) {}
                }
                WontonButton(
                    stringResource(R.string.family_inviteSubmit),
                    {
                        val action = FamilyAction.InviteMember(
                            displayName = name.trim(),
                            username = username.trim().ifEmpty { null },
                            privateBot = privateBot && role?.permissions?.privateBot == true,
                            roleId = if (view.access.choosesRole()) roleId else null,
                        )
                        core.send(Action.Family(action))?.let(onRejection)
                    },
                    Modifier.fillMaxWidth(),
                    enabled = canSubmit,
                )
            }
        }
    }
}

@Composable
private fun RolePicker(
    view: FamilyViewModel,
    selected: RoleDef?,
    open: Boolean,
    onOpen: (Boolean) -> Unit,
    onSelect: (String) -> Unit,
) {
    WontonRow(
        title = stringResource(R.string.family_role),
        value = selected?.let { roleName(it.name, it.template, it.id) },
        showsForward = true,
        onClick = { onOpen(true) },
    )
    DropdownMenu(open, onDismissRequest = { onOpen(false) }) {
        view.roles.roles.forEach { role ->
            DropdownMenuItem(
                text = { Text(roleName(role.name, role.template, role.id)) },
                onClick = { onSelect(role.id); onOpen(false) },
            )
        }
    }
}

@Composable
private fun InviteCodePanel(
    core: ClientCore,
    view: FamilyViewModel,
    onDone: () -> Unit,
    close: () -> Unit,
) {
    val invite = view.invite ?: return
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        Text(stringResource(R.string.family_codeTitle, invite.displayName))
        Image(
            qrBitmap(invite.code),
            contentDescription = stringResource(R.string.family_qrCode),
            modifier = Modifier.size(200.dp),
        )
        Text(codeRemaining(app.wonton.protocol.MemberCode(invite.expiresAtMs, false)))
        WontonButton(stringResource(R.string.family_codeShare), {
            val send = Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"
                putExtra(Intent.EXTRA_TEXT, invite.code)
            }
            context.startActivity(Intent.createChooser(send, null))
        }, Modifier.fillMaxWidth(), kind = WontonButtonKind.SECONDARY)
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(stringResource(R.string.family_resendCode), {
                core.send(Action.Family(FamilyAction.ResendMemberCode(invite.username)))
            }, kind = WontonButtonKind.SECONDARY)
            WontonButton(stringResource(R.string.family_revokeCode), {
                core.send(Action.Family(FamilyAction.DismissMemberInvite))
                close()
            }, kind = WontonButtonKind.SECONDARY)
        }
        WontonButton(stringResource(R.string.common_done), {
            core.send(Action.Family(FamilyAction.DismissMemberInvite))
            onDone()
        }, Modifier.fillMaxWidth())
    }
}

@Composable
fun MemberRolePicker(core: ClientCore, view: FamilyViewModel, member: MemberRow, onDismiss: () -> Unit) {
    WontonSheet(onDismiss, stringResource(R.string.family_changeRole)) { close ->
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            view.roles.roles.forEach { role ->
                WontonRow(
                    title = roleName(role.name, role.template, role.id),
                    value = stringResource(R.string.family_rolePeople, role.people),
                    titleTint = if (role.id == member.roleId) WontonTheme.colors.primary else null,
                    onClick = {
                        core.send(Action.Family(FamilyAction.AssignMemberRole(member.username, role.id)))
                        close {}
                    },
                )
            }
        }
    }
}

@Composable
fun MemberRemoveSheet(core: ClientCore, member: MemberRow, onDismiss: () -> Unit) {
    var deleteHome by remember { mutableStateOf(true) }
    var keepBotData by remember { mutableStateOf(true) }
    WontonSheet(onDismiss, stringResource(R.string.family_removeTitle, member.displayName)) { close ->
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            Text(stringResource(R.string.family_removeBlurb, member.displayName))
            ToggleLine(
                title = stringResource(R.string.family_removeDeleteHome),
                hint = stringResource(R.string.family_removeDeleteHomeHint),
                checked = deleteHome,
                onChange = { deleteHome = it },
            )
            ToggleLine(
                title = stringResource(R.string.family_removeKeepBotData),
                hint = stringResource(R.string.family_removeKeepBotDataHint),
                checked = keepBotData,
                onChange = { keepBotData = it },
            )
            WontonButton(stringResource(R.string.family_removeMember), {
                core.send(
                    Action.Family(FamilyAction.RemoveMember(member.username, deleteHome, keepBotData)),
                )
                close {}
            }, Modifier.fillMaxWidth(), kind = WontonButtonKind.DANGER)
        }
    }
}

@Composable
private fun ToggleLine(title: String, hint: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().toggleable(checked, role = Role.Switch, onValueChange = onChange)
            .padding(WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title)
            Text(hint, style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
        }
        Switch(checked, onCheckedChange = null)
    }
}
