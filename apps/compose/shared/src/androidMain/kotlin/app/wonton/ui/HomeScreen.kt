package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.HomeFailure
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PairingStatus

/** “家”：还没配对时是配对页，配上以后是和家里服务器的连接状态。只渲染核心的 ViewModel。 */
@Composable
fun HomeRoute(core: ClientCore, viewModel: HomeViewModel) {
    var pairingAgain by rememberSaveable { mutableStateOf(false) }
    var inviting by rememberSaveable { mutableStateOf(false) }
    BackHandler(enabled = pairingAgain || inviting) {
        pairingAgain = false
        inviting = false
    }
    when {
        viewModel.status is HomeStatus.NotConfigured -> PairingPage("家", core, viewModel.pairing, onBack = null)
        pairingAgain -> PairingPage("重新配对", core, viewModel.pairing, onBack = { pairingAgain = false })
        inviting -> InvitePage(core, viewModel, onDone = { inviting = false })
        else -> HomeScreen(viewModel, onPairAgain = { pairingAgain = true }, onInvite = { inviting = true })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InvitePage(core: ClientCore, viewModel: HomeViewModel, onDone: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("添加设备") },
                navigationIcon = { TextButton(onClick = onDone) { Text("完成") } },
            )
        },
    ) { padding ->
        InviteScreen(core, viewModel.invite, Modifier.padding(padding))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PairingPage(title: String, core: ClientCore, pairing: PairingStatus?, onBack: (() -> Unit)?) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = { onBack?.let { TextButton(onClick = it) { Text("返回") } } },
            )
        },
    ) { padding ->
        PairingScreen(core, pairing, Modifier.padding(padding), onPaired = { onBack?.invoke() })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeScreen(viewModel: HomeViewModel, onPairAgain: () -> Unit, onInvite: () -> Unit) {
    Scaffold(topBar = { TopAppBar(title = { Text("家") }) }) { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize()) {
            Text("状态：${statusText(viewModel.status)}")
            viewModel.path?.let { path ->
                Text("路径：${if (path.relay) "中继" else "直连"} · ${path.rttMs} ms")
            }
            TextButton(onClick = onInvite, Modifier.padding(top = 24.dp)) { Text("添加设备") }
            TextButton(onClick = onPairAgain) { Text("重新配对") }
        }
    }
}

private fun statusText(status: HomeStatus): String = when (status) {
    is HomeStatus.NotConfigured -> "还没有配对"
    is HomeStatus.Connecting -> "连接中"
    is HomeStatus.Connected -> "已连接"
    is HomeStatus.Disconnected -> when (status.reason) {
        HomeFailure.UNAUTHORIZED -> "家里的服务器不认这台设备，请重新扫码配对"
        else -> status.message
    }
}
