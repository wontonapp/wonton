package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.HomeFailure
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel

/** “家”：渲染核心的连接 ViewModel；网页入口和本地代理都就绪时可以打开插件网页。 */
@Composable
fun HomeRoute(core: ClientCore, viewModel: HomeViewModel, onOpenWeb: () -> Unit) {
    var rejection by remember { mutableStateOf<String?>(null) }
    HomeScreen(
        viewModel = viewModel,
        rejection = rejection,
        onAction = { action -> rejection = core.send(action) },
        onOpenWeb = onOpenWeb,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeScreen(
    viewModel: HomeViewModel,
    rejection: String?,
    onAction: (Action) -> Unit,
    onOpenWeb: () -> Unit,
) {
    var draftTicket by rememberSaveable { mutableStateOf("") }
    Scaffold(topBar = { TopAppBar(title = { Text("家") }) }) { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize()) {
            Text("设备 ID", style = MaterialTheme.typography.labelMedium)
            SelectionContainer {
                Text(viewModel.deviceId, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
            }
            Text("状态：${statusText(viewModel.status)}", Modifier.padding(top = 12.dp))
            viewModel.path?.let { path ->
                Text("路径：${if (path.relay) "中继" else "直连"} · ${path.rttMs} ms")
            }
            viewModel.proxy?.let { proxy -> Text("本地代理：${proxy.host}:${proxy.port}") }
            if (viewModel.webUrl != null && viewModel.proxy != null) {
                Button(onClick = onOpenWeb, Modifier.padding(top = 12.dp)) { Text("打开 ${viewModel.webUrl}") }
            }
            OutlinedTextField(
                value = draftTicket,
                onValueChange = { draftTicket = it },
                label = { Text("连接信息（dev.sh ticket 的输出）") },
                modifier = Modifier.padding(top = 24.dp),
            )
            Button(onClick = { onAction(Action.ConnectHome(draftTicket)) }, enabled = draftTicket.isNotEmpty()) {
                Text("连接")
            }
            rejection?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}

private fun statusText(status: HomeStatus): String = when (status) {
    is HomeStatus.NotConfigured -> "还没有连接信息"
    is HomeStatus.Connecting -> "连接中"
    is HomeStatus.Connected -> "已连接"
    is HomeStatus.Disconnected -> when (status.reason) {
        HomeFailure.UNAUTHORIZED -> "服务端不认这台设备，请主人先登记它"
        else -> status.message
    }
}
