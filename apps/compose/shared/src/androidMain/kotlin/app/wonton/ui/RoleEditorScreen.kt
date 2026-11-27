package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AdminScope
import app.wonton.protocol.AiLevel
import app.wonton.protocol.AppScope
import app.wonton.protocol.FamilyAction
import app.wonton.protocol.FamilyViewModel
import app.wonton.protocol.Gate
import app.wonton.protocol.Permissions
import app.wonton.protocol.RoleDef
import app.wonton.protocol.RoleDraft
import app.wonton.protocol.RoleTemplate
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private enum class GateChoice(val labelRes: Int) {
    ALLOW(R.string.family_gateAllow),
    ASK_ADMIN(R.string.family_gateAskAdmin),
    DENY(R.string.family_gateDeny),
}

private enum class AdminChoice(val labelRes: Int) {
    ALL(R.string.family_adminAll),
    SELECTED(R.string.family_adminSelected),
    NONE(R.string.family_adminNone),
}

private enum class AiChoice(val labelRes: Int) {
    READ_ONLY(R.string.family_aiReadOnly),
    ASK_ADMIN(R.string.family_aiAskAdmin),
    SELF(R.string.family_aiSelf),
}

@Suppress("LongMethod", "CyclomaticComplexMethod")
@Composable
fun RoleEditorSheet(
    core: ClientCore,
    plugins: PluginsState,
    view: FamilyViewModel,
    role: RoleDef?,
    onDismiss: () -> Unit,
    onRejection: (Problem) -> Unit,
) {
    val initialName = role?.let { roleName(it.name, it.template, it.id) } ?: ""
    var name by remember(role) { mutableStateOf(initialName) }
    var openAppsAll by remember(role) { mutableStateOf(role?.permissions?.openApps == AppScope.All) }
    var openAppsSelected by remember(role) {
        mutableStateOf((role?.permissions?.openApps as? AppScope.Selected)?.pluginIds?.toSet() ?: emptySet())
    }
    var admin by remember(role) {
        mutableStateOf(
            when (role?.permissions?.pluginAdmin) {
                AdminScope.AllOpenable -> AdminChoice.ALL
                is AdminScope.Selected -> AdminChoice.SELECTED
                else -> AdminChoice.NONE
            },
        )
    }
    var adminSelected by remember(role) {
        mutableStateOf((role?.permissions?.pluginAdmin as? AdminScope.Selected)?.pluginIds?.toSet() ?: emptySet())
    }
    var install by remember(role) {
        mutableStateOf(role?.permissions?.installApps?.let { it.gateChoice() } ?: GateChoice.DENY)
    }
    var modelSettings by remember(role) { mutableStateOf(role?.permissions?.modelSettings == true) }
    var serverSettings by remember(role) { mutableStateOf(role?.permissions?.serverSettings == true) }
    var manageMembers by remember(role) { mutableStateOf(role?.permissions?.manageMembers == true) }
    var addFriends by remember(role) {
        mutableStateOf(role?.permissions?.addFriends?.let { it.gateChoice() } ?: GateChoice.DENY)
    }
    var shareApps by remember(role) { mutableStateOf(role?.permissions?.shareApps == true) }
    var ai by remember(role) {
        mutableStateOf(
            when (role?.permissions?.ai) {
                AiLevel.ADMIN_APPROVES_DELETES -> AiChoice.ASK_ADMIN
                AiLevel.SELF_CONFIRMS -> AiChoice.SELF
                else -> AiChoice.READ_ONLY
            },
        )
    }
    var privileged by remember(role) { mutableStateOf(role?.permissions?.privilegedTools == true) }
    var privateBot by remember(role) { mutableStateOf(role?.permissions?.privateBot == true) }
    var sharedBots by remember(role) { mutableStateOf(role?.permissions?.sharedBots == true) }
    var lendOwnBot by remember(role) { mutableStateOf(role?.permissions?.lendOwnBot == true) }

    fun draft(): Permissions = Permissions(
        openApps = if (openAppsAll) AppScope.All else AppScope.Selected(openAppsSelected.toList()),
        pluginAdmin = when (admin) {
            AdminChoice.ALL -> AdminScope.AllOpenable
            AdminChoice.SELECTED -> AdminScope.Selected(adminSelected.toList())
            AdminChoice.NONE -> AdminScope.NoApps
        },
        installApps = install.gate(),
        modelSettings = modelSettings,
        serverSettings = serverSettings,
        manageMembers = manageMembers,
        addFriends = addFriends.gate(),
        shareApps = shareApps,
        ai = when (ai) {
            AiChoice.READ_ONLY -> AiLevel.READ_ONLY
            AiChoice.ASK_ADMIN -> AiLevel.ADMIN_APPROVES_DELETES
            AiChoice.SELF -> AiLevel.SELF_CONFIRMS
        },
        privilegedTools = privileged,
        privateBot = privateBot,
        sharedBots = sharedBots,
        lendOwnBot = lendOwnBot,
    )

    val sheetTitle = role?.let { roleName(it.name, it.template, it.id) }
        ?: stringResource(R.string.family_roleNew)
    WontonSheet(onDismiss, sheetTitle) { close ->
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonTextField(
                value = name,
                onValueChange = { name = it },
                modifier = Modifier.fillMaxWidth(),
                label = stringResource(R.string.family_roleName),
            )
            if (role == null) {
                SectionText(stringResource(R.string.family_roleStartFrom))
                view.roles.roles.forEach { template ->
                    val templateTemplate = template.template ?: return@forEach
                    val templateRoleName = roleName(template.name, template.template, template.id)
                    WontonButton(templateName(templateTemplate), {
                        name = templateRoleName
                        val p = template.permissions
                        openAppsAll = p.openApps == AppScope.All
                        openAppsSelected = (p.openApps as? AppScope.Selected)?.pluginIds?.toSet() ?: emptySet()
                        admin = when (p.pluginAdmin) {
                            AdminScope.AllOpenable -> AdminChoice.ALL
                            is AdminScope.Selected -> AdminChoice.SELECTED
                            else -> AdminChoice.NONE
                        }
                        adminSelected = (p.pluginAdmin as? AdminScope.Selected)?.pluginIds?.toSet() ?: emptySet()
                        install = p.installApps.gateChoice()
                        modelSettings = p.modelSettings
                        serverSettings = p.serverSettings
                        manageMembers = p.manageMembers
                        addFriends = p.addFriends.gateChoice()
                        shareApps = p.shareApps
                        ai = when (p.ai) {
                            AiLevel.ADMIN_APPROVES_DELETES -> AiChoice.ASK_ADMIN
                            AiLevel.SELF_CONFIRMS -> AiChoice.SELF
                            else -> AiChoice.READ_ONLY
                        }
                        privileged = p.privilegedTools
                        privateBot = p.privateBot
                        sharedBots = p.sharedBots
                        lendOwnBot = p.lendOwnBot
                    })
                }
            }
            WontonGroup {
                Column(
                    Modifier.padding(WontonTheme.spacing.s12),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    Row(
                        Modifier.fillMaxWidth().toggleable(openAppsAll, role = Role.Switch) { openAppsAll = it },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(stringResource(R.string.family_permissionOpenApps), Modifier.weight(1f))
                        Switch(openAppsAll, onCheckedChange = null)
                    }
                    if (!openAppsAll) AppToggleList(plugins, openAppsSelected) { openAppsSelected = it }
                }
                WontonSeparator()
                Column(
                    Modifier.padding(WontonTheme.spacing.s12),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    Text(stringResource(R.string.family_permissionPluginAdmin))
                    ChoiceRow(AdminChoice.entries.map { it to stringResource(it.labelRes) }, admin) { admin = it }
                    if (admin == AdminChoice.SELECTED) AppToggleList(plugins, adminSelected) { adminSelected = it }
                }
                WontonSeparator()
                GateRow(stringResource(R.string.family_permissionInstallApps), install) { install = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionModelSettings), modelSettings) { modelSettings = it }
                WontonSeparator()
                BoolRow(
                    stringResource(R.string.family_permissionServerSettings),
                    serverSettings,
                ) { serverSettings = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionManageMembers), manageMembers) { manageMembers = it }
                WontonSeparator()
                GateRow(stringResource(R.string.family_permissionAddFriends), addFriends) { addFriends = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionShareApps), shareApps) { shareApps = it }
                WontonSeparator()
                Column(
                    Modifier.padding(WontonTheme.spacing.s12),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    Text(stringResource(R.string.family_permissionAi))
                    ChoiceRow(AiChoice.entries.map { it to stringResource(it.labelRes) }, ai) { ai = it }
                }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionPrivilegedTools), privileged) { privileged = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionPrivateBot), privateBot) { privateBot = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionSharedBots), sharedBots) { sharedBots = it }
                WontonSeparator()
                BoolRow(stringResource(R.string.family_permissionLendOwnBot), lendOwnBot) { lendOwnBot = it }
            }
            view.roles.problem?.let { ProblemRow(it.problem) }
            WontonButton(stringResource(R.string.family_roleSave), {
                val draft = RoleDraft(name.trim(), draft())
                val action = if (role == null) {
                    FamilyAction.CreateRole(draft)
                } else {
                    FamilyAction.UpdateRole(role.id, draft)
                }
                core.send(Action.Family(action))?.let(onRejection)
                close {}
            }, Modifier.fillMaxWidth(), enabled = name.isNotBlank())
            if (role != null && role.people == 0u && role.id != view.roles.defaultInviteRole) {
                WontonButton(stringResource(R.string.family_removeMember), {
                    core.send(Action.Family(FamilyAction.DeleteRole(role.id)))?.let(onRejection)
                    close {}
                }, Modifier.fillMaxWidth(), kind = WontonButtonKind.SECONDARY)
            }
        }
    }
}

@Composable
private fun SectionText(text: String) {
    Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
}

@Composable
private fun BoolRow(title: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().toggleable(checked, role = Role.Switch) { onChange(it) }
            .padding(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f))
        Switch(checked, onCheckedChange = null)
    }
}

@Composable
private fun GateRow(title: String, choice: GateChoice, onChange: (GateChoice) -> Unit) {
    Column(
        Modifier.padding(WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(title)
        ChoiceRow(GateChoice.entries.map { it to stringResource(it.labelRes) }, choice, onChange)
    }
}

@Composable
private fun <T> ChoiceRow(options: List<Pair<T, String>>, selected: T, onSelect: (T) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        options.forEach { (value, label) ->
            FilterChip(selected = value == selected, onClick = { onSelect(value) }, label = { Text(label) })
        }
    }
}

@Composable
private fun AppToggleList(plugins: PluginsState, selected: Set<String>, onChange: (Set<String>) -> Unit) {
    val installed = plugins.viewModel.installed?.plugins.orEmpty()
    if (installed.isEmpty()) {
        Text(
            stringResource(R.string.family_roleNoAppsInstalled),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    } else {
        installed.forEach { plugin ->
            val checked = plugin.id in selected
            Row(
                Modifier.fillMaxWidth().toggleable(checked, role = Role.Switch) {
                    onChange(if (it) selected + plugin.id else selected - plugin.id)
                }.padding(vertical = WontonTheme.spacing.s4),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(plugin.title, Modifier.weight(1f))
                Switch(checked, onCheckedChange = null)
            }
        }
    }
}

private fun Gate.gateChoice(): GateChoice = when (this) {
    Gate.ALLOW -> GateChoice.ALLOW
    Gate.ASK_ADMIN -> GateChoice.ASK_ADMIN
    else -> GateChoice.DENY
}

private fun GateChoice.gate(): Gate = when (this) {
    GateChoice.ALLOW -> Gate.ALLOW
    GateChoice.ASK_ADMIN -> Gate.ASK_ADMIN
    GateChoice.DENY -> Gate.DENY
}
