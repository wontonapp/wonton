package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.HomeAction
import app.wonton.protocol.Action
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PairingStatus
import app.wonton.protocol.ServerName
import app.wonton.protocol.ServerRow
import app.wonton.protocol.ServersViewModel
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Globe
import app.wonton.ui.icons.WontonIcon

internal val LocalServers = compositionLocalOf<ServersViewModel> { error("Servers are not provided") }

@Composable
internal fun ServerName.displayName(): String = when (this) {
    is ServerName.Named -> name
    is ServerName.Default -> if (number == 1u) {
        stringResource(R.string.servers_default_first)
    } else {
        pluralStringResource(R.plurals.servers_default_numbered, number.toInt(), number.toLong())
    }
}

@Composable
internal fun ServersGroup(plugins: PluginsState, onMatrixLogin: () -> Unit) {
    val view = LocalServers.current
    var rename by remember { mutableStateOf<ServerRow?>(null) }
    var remove by remember { mutableStateOf<ServerRow?>(null) }
    WontonGroup {
        view.servers.forEach { server ->
            ServerItem(server, plugins::send, { rename = server }, { remove = server })
            WontonSeparator()
        }
        AddServerRow { plugins.send(Action.Home(HomeAction.AddServer)) }
        WontonSeparator()
        MatrixLoginRow(onMatrixLogin)
    }
    rename?.let { server -> RenameServerDialog(server, plugins::send) { rename = null } }
    remove?.let { server ->
        val dismiss = { remove = null }
        WontonDialog(
            stringResource(R.string.servers_remove_title, server.name.displayName()),
            stringResource(R.string.servers_remove_message),
            dismiss,
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
                WontonDialogAction(stringResource(R.string.common_remove), WontonButtonKind.DANGER) {
                    dismiss()
                    plugins.send(Action.Home(HomeAction.RemoveServer(server.id)))
                },
            ),
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ServerItem(server: ServerRow, send: (Action) -> Unit, rename: () -> Unit, remove: () -> Unit) {
    var menu by remember(server.id) { mutableStateOf(false) }
    Box {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                .semantics { selected = server.current }
                .combinedClickable(
                    role = Role.Button,
                    onClick = { if (!server.current) send(Action.Home(HomeAction.SwitchServer(server.id))) },
                    onLongClick = { menu = true },
                    onLongClickLabel = stringResource(R.string.servers_manage),
                )
                .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                server.name.displayName(), Modifier.weight(1f), style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            if (server.current) Icon(
                Icons.Default.Check,
                // 装饰：这一行有 selected 状态。
                contentDescription = null,
                tint = WontonTheme.colors.primary.toColor(),
            )
        }
        DropdownMenu(menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.common_rename)) },
                onClick = { menu = false; rename() },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.common_remove), color = WontonTheme.colors.danger.toColor()) },
                onClick = { menu = false; remove() },
            )
        }
    }
}

@Composable
private fun RenameServerDialog(server: ServerRow, send: (Action) -> Unit, dismiss: () -> Unit) {
    val initialName = server.name.displayName()
    var name by remember(server.id) { mutableStateOf(initialName) }
    WontonDialog(
        stringResource(R.string.common_rename), onDismiss = dismiss,
        actions = listOf(WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss)),
        detail = {
            WontonTextField(name, { name = it }, Modifier.fillMaxWidth(), label = stringResource(R.string.servers_name))
            WontonButton(
                stringResource(R.string.common_save), {
                    send(Action.Home(HomeAction.RenameServer(server.id, name)))
                    dismiss()
                },
                Modifier.fillMaxWidth(), enabled = name.isNotBlank(),
            )
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AddServerPairing(plugins: PluginsState, home: HomeViewModel) {
    val cancel = { plugins.send(Action.Home(HomeAction.CancelAddServer)) }
    val back = {
        if (home.pairing is PairingStatus.Failed) plugins.send(Action.Home(HomeAction.DismissPairingFailure))
        cancel()
    }
    BackHandler(onBack = back)
    WontonScreen(bar = {
        TopAppBar(
            title = { Text(stringResource(R.string.servers_add_title), style = WontonTheme.typography.itemTitle) },
            navigationIcon = { TextButton(onClick = back) { Text(stringResource(R.string.common_cancel)) } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = WontonTheme.colors.canvas.toColor()),
        )
    }) {
        PairingScreen(plugins.core, home.pairing, onBack = cancel)
    }
}

@Composable
private fun AddServerRow(add: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = add)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Default.Add,
            // 装饰：右边的文字就是“添加服务器”。
            contentDescription = null,
            tint = WontonTheme.colors.primary.toColor(),
        )
        Text(
            stringResource(R.string.servers_add), Modifier.padding(start = WontonTheme.spacing.s12),
            style = WontonTheme.typography.body, color = WontonTheme.colors.primary.toColor(),
        )
    }
}

@Composable
private fun MatrixLoginRow(open: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = open)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            WontonIcon.Globe,
            // 装饰：右边的文字就是“用 Matrix 账号登录”。
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.primary.toColor(),
        )
        Text(
            stringResource(R.string.matrix_login_entry), Modifier.padding(start = WontonTheme.spacing.s12),
            style = WontonTheme.typography.body, color = WontonTheme.colors.primary.toColor(),
        )
    }
}
