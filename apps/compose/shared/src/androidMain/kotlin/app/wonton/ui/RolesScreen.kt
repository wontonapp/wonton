package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import app.wonton.protocol.FamilyAction
import app.wonton.protocol.RoleDef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.WontonIcon

/** 角色页（ADR-0088）：只有主人能进；新建、改名、逐项权限、默认角色。 */
@Composable
fun RolesPage(core: ClientCore, plugins: PluginsState, onBack: () -> Unit) {
    val view by remember(core) { core.familyUpdates() }.collectAsState(remember(core) { core.family() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var editing by remember { mutableStateOf<RoleDef?>(null) }
    var creating by remember { mutableStateOf(false) }
    val send = { action: Action -> rejection = core.send(action) }
    LaunchedEffect(core) { send(Action.Family(FamilyAction.RefreshRoles)) }
    DisposableEffect(core) { onDispose {} }
    BackHandler(onBack = onBack)
    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.family_rolesTitle), onBack, actions = {
            WontonIconButton(WontonIcon.Add, stringResource(R.string.family_roleNew), { creating = true })
        })
    }) {
        Column(
            Modifier.fillMaxSize().wontonContentPadding().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            view.roles.problem?.let { problem ->
                ProblemRow(problem.problem) { send(Action.Family(FamilyAction.RefreshRoles)) }
            }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                SectionText(stringResource(R.string.family_roleList))
                if (view.roles.roles.isEmpty()) {
                    SectionText(stringResource(R.string.family_roleEmpty))
                } else {
                    WontonGroup {
                        view.roles.roles.forEachIndexed { index, role ->
                            if (index > 0) WontonSeparator()
                            RoleRow(role) { editing = role }
                        }
                    }
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                SectionText(stringResource(R.string.family_defaultRole))
                WontonGroup {
                    view.roles.roles.forEach { role ->
                        WontonRow(
                            title = roleName(role.name, role.template, role.id),
                            value = if (role.id == view.roles.defaultInviteRole) "✓" else null,
                            onClick = {
                                if (role.id != view.roles.defaultInviteRole) {
                                    send(Action.Family(FamilyAction.SetDefaultRole(role.id)))
                                }
                            },
                        )
                    }
                }
            }
        }
    }
    if (creating) {
        RoleEditorSheet(core, plugins, view, null, onDismiss = { creating = false }, onRejection = { rejection = it })
    }
    editing?.let { role ->
        RoleEditorSheet(core, plugins, view, role, onDismiss = { editing = null }, onRejection = { rejection = it })
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

@Composable
private fun SectionText(text: String) {
    androidx.compose.material3.Text(
        text,
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

@Composable
private fun RoleRow(role: RoleDef, onClick: () -> Unit) {
    WontonRow(
        title = roleName(role.name, role.template, role.id),
        value = stringResource(R.string.family_rolePeople, role.people),
        showsForward = true,
        onClick = onClick,
    )
}
