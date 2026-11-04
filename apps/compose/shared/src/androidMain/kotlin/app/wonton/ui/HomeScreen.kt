package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.HomeFailure
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PairingStatus
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme

/**
 * “应用”：还没配对时是配对页；配上以后按图标和名字列出已装的插件，最后一格进应用商店，连接和设备在右上角。
 * 只渲染核心的 ViewModel。
 */
@Composable
fun HomeRoute(
    core: ClientCore,
    viewModel: HomeViewModel,
    plugins: PluginsState,
    navigation: PluginNavigation,
    page: HomePage,
    onPage: (HomePage) -> Unit,
    pendingCards: List<PendingConfirmation>,
    onOpenCardRoom: (String) -> Unit,
) {
    BackHandler(enabled = page != HomePage.LAUNCHER) {
        onPage(
            when (page) {
                HomePage.CONNECTION, HomePage.STORE -> HomePage.LAUNCHER
                else -> HomePage.CONNECTION
            },
        )
    }
    when {
        viewModel.status is HomeStatus.NotConfigured ->
            PairingPage("配对", core, viewModel.pairing, onBack = null)
        page == HomePage.PAIRING ->
            PairingPage("重新配对", core, viewModel.pairing, onBack = { onPage(HomePage.CONNECTION) })
        page == HomePage.INVITE -> InvitePage(core, viewModel, onBack = { onPage(HomePage.CONNECTION) })
        page == HomePage.CONNECTION -> ConnectionPage(viewModel, onNavigate = onPage)
        page == HomePage.EGRESS -> EgressPage(plugins, onBack = { onPage(HomePage.CONNECTION) })
        else -> PluginPages(plugins, navigation) {
            if (page == HomePage.STORE) {
                StoreRoute(plugins, navigation, onBack = { onPage(HomePage.LAUNCHER) })
            } else {
                LauncherScreen(
                    viewModel,
                    plugins,
                    navigation,
                    pendingCards,
                    onOpenCardRoom,
                    onShowStore = { onPage(HomePage.STORE) },
                    onConnection = { onPage(HomePage.CONNECTION) },
                )
            }
        }
    }
}

enum class HomePage { LAUNCHER, CONNECTION, INVITE, PAIRING, EGRESS, STORE }

@Composable
private fun InvitePage(core: ClientCore, viewModel: HomeViewModel, onBack: () -> Unit) {
    WontonScreen(bar = { WontonPushedBar("添加设备", onBack) }) {
        InviteScreen(core, viewModel.invite)
    }
}

@Composable
private fun PairingPage(
    title: String,
    core: ClientCore,
    pairing: PairingStatus?,
    onBack: (() -> Unit)?,
) {
    WontonScreen(
        bar = {
            if (onBack != null) WontonPushedBar(title, onBack) else WontonRootBar(title)
        },
    ) {
        PairingScreen(core, pairing, onPaired = { onBack?.invoke() })
    }
}

/** 和家里服务器的连接，以及添加设备、重新配对。 */
@Composable
private fun ConnectionPage(viewModel: HomeViewModel, onNavigate: (HomePage) -> Unit) {
    WontonScreen(bar = { WontonPushedBar("连接", { onNavigate(HomePage.LAUNCHER) }) }) {
        Column(
            Modifier.fillMaxSize().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                WontonRow("状态", viewModel.status.text)
                viewModel.path?.let { path ->
                    WontonSeparator()
                    WontonRow("路径", "${if (path.relay) "中继" else "直连"} · ${path.rttMs} ms")
                }
            }
            WontonGroup {
                WontonRow("出网代理", showsForward = true, onClick = { onNavigate(HomePage.EGRESS) })
            }
            WontonGroup {
                WontonRow("添加设备", showsForward = true, onClick = { onNavigate(HomePage.INVITE) })
                WontonSeparator()
                WontonRow("重新配对", showsForward = true, onClick = { onNavigate(HomePage.PAIRING) })
            }
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
