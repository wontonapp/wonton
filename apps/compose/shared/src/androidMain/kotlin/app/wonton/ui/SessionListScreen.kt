package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.SessionListViewModel
import app.wonton.protocol.SessionRow

/** 会话列表页：订阅核心的 ViewModel 并渲染，用户操作转成 [Action] 发回核心。 */
@Composable
fun SessionListRoute(core: ClientCore) {
    val updates = remember(core) { core.sessionListUpdates() }
    val initial = remember(core) { core.sessionList() }
    val viewModel by updates.collectAsState(initial = initial)
    var rejection by remember { mutableStateOf<String?>(null) }

    SessionListScreen(
        viewModel = viewModel,
        rejection = rejection,
        onAction = { action -> rejection = core.send(action) },
        onDismissRejection = { rejection = null },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionListScreen(
    viewModel: SessionListViewModel,
    rejection: String?,
    onAction: (Action) -> Unit,
    onDismissRejection: () -> Unit,
) {
    var isNamingSession by rememberSaveable { mutableStateOf(false) }

    Scaffold(
        topBar = { TopAppBar(title = { Text("会话") }) },
        floatingActionButton = {
            ExtendedFloatingActionButton(onClick = { isNamingSession = true }) { Text("新建会话") }
        },
    ) { padding ->
        if (viewModel.sessions.isEmpty()) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                Text("还没有会话")
            }
        } else {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = padding) {
                items(viewModel.sessions, key = { it.id }) { row -> SessionRowItem(row, onAction) }
            }
        }
    }

    if (isNamingSession) {
        NewSessionDialog(
            onCreate = { title ->
                isNamingSession = false
                onAction(Action.CreateSession(title))
            },
            onDismiss = { isNamingSession = false },
        )
    }

    if (rejection != null) {
        AlertDialog(
            onDismissRequest = onDismissRejection,
            confirmButton = { TextButton(onClick = onDismissRejection) { Text("好") } },
            title = { Text("操作被拒绝") },
            text = { Text(rejection) },
        )
    }
}

@Composable
private fun SessionRowItem(row: SessionRow, onAction: (Action) -> Unit) {
    val select = { onAction(Action.SelectSession(row.id)) }
    ListItem(
        headlineContent = { Text(row.title) },
        leadingContent = { RadioButton(selected = row.isSelected, onClick = select) },
        trailingContent = {
            TextButton(onClick = { onAction(Action.DeleteSession(row.id)) }) { Text("删除") }
        },
        modifier = Modifier.clickable(onClick = select),
    )
}

@Composable
private fun NewSessionDialog(onCreate: (String) -> Unit, onDismiss: () -> Unit) {
    var title by rememberSaveable { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("新建会话") },
        text = {
            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                label = { Text("标题") },
                singleLine = true,
            )
        },
        confirmButton = { TextButton(onClick = { onCreate(title) }) { Text("创建") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
    )
}
