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
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.HomeFailure
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PairingStatus

/**
 * “家”：还没配对时是配对页；配上以后是主屏，按图标和名字列出已装的插件，连接和设备在右上角。
 * 只渲染核心的 ViewModel。
 */
@Composable
fun HomeRoute(
    core: ClientCore,
    viewModel: HomeViewModel,
    plugins: PluginsState,
    navigation: PluginNavigation,
    appearance: AppearanceViewModel,
    onShowStore: () -> Unit,
) {
    var page by rememberSaveable { mutableStateOf(HomePage.LAUNCHER) }
    BackHandler(enabled = page != HomePage.LAUNCHER) {
        page = when (page) {
            HomePage.CONNECTION, HomePage.APPEARANCE -> HomePage.LAUNCHER
            else -> HomePage.CONNECTION
        }
    }
    when {
        // 外观不依赖家里的服务器，没配对时也能进。
        page == HomePage.APPEARANCE -> AppearancePage(core, appearance, onBack = { page = HomePage.LAUNCHER })
        viewModel.status is HomeStatus.NotConfigured ->
            PairingPage("家", core, viewModel.pairing, onBack = null, onAppearance = { page = HomePage.APPEARANCE })
        page == HomePage.PAIRING ->
            PairingPage("重新配对", core, viewModel.pairing, onBack = { page = HomePage.CONNECTION })
        page == HomePage.INVITE -> InvitePage(core, viewModel, onDone = { page = HomePage.CONNECTION })
        page == HomePage.CONNECTION -> ConnectionPage(viewModel, onNavigate = { page = it })
        else -> PluginPages(plugins, navigation) {
            LauncherScreen(
                viewModel.status,
                plugins,
                navigation,
                onShowStore,
                onAppearance = { page = HomePage.APPEARANCE },
                onConnection = { page = HomePage.CONNECTION },
            )
        }
    }
}

enum class HomePage { LAUNCHER, CONNECTION, INVITE, PAIRING, APPEARANCE }

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
private fun PairingPage(
    title: String,
    core: ClientCore,
    pairing: PairingStatus?,
    onBack: (() -> Unit)?,
    onAppearance: (() -> Unit)? = null,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = { onBack?.let { TextButton(onClick = it) { Text("返回") } } },
                actions = { onAppearance?.let { TextButton(onClick = it) { Text("外观") } } },
            )
        },
    ) { padding ->
        PairingScreen(core, pairing, Modifier.padding(padding), onPaired = { onBack?.invoke() })
    }
}

/** 和家里服务器的连接，以及添加设备、重新配对。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConnectionPage(viewModel: HomeViewModel, onNavigate: (HomePage) -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("连接") },
                navigationIcon = { TextButton(onClick = { onNavigate(HomePage.LAUNCHER) }) { Text("返回") } },
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize()) {
            Text("状态：${viewModel.status.text}")
            viewModel.path?.let { path ->
                Text("路径：${if (path.relay) "中继" else "直连"} · ${path.rttMs} ms")
            }
            TextButton(onClick = { onNavigate(HomePage.INVITE) }, Modifier.padding(top = 24.dp)) { Text("添加设备") }
            TextButton(onClick = { onNavigate(HomePage.PAIRING) }) { Text("重新配对") }
        }
    }
}

val HomeStatus.text: String
    get() = when (this) {
        is HomeStatus.NotConfigured -> "还没有配对"
        is HomeStatus.Connecting -> "连接中"
        is HomeStatus.Connected -> "已连接"
        is HomeStatus.Disconnected -> when (reason) {
            HomeFailure.UNAUTHORIZED -> "家里的服务器不认这台设备，请重新扫码配对"
            else -> message
        }
    }
