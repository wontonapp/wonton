package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
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
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Palette
import app.wonton.ui.icons.WontonIcon

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
            PairingPage("配对", core, viewModel.pairing, onBack = null, onAppearance = { page = HomePage.APPEARANCE })
        page == HomePage.PAIRING ->
            PairingPage("重新配对", core, viewModel.pairing, onBack = { page = HomePage.CONNECTION })
        page == HomePage.INVITE -> InvitePage(core, viewModel, onBack = { page = HomePage.CONNECTION })
        page == HomePage.CONNECTION -> ConnectionPage(viewModel, onNavigate = { page = it })
        page == HomePage.EGRESS -> EgressPage(plugins, onBack = { page = HomePage.CONNECTION })
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

enum class HomePage { LAUNCHER, CONNECTION, INVITE, PAIRING, APPEARANCE, EGRESS }

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
    onAppearance: (() -> Unit)? = null,
) {
    WontonScreen(
        bar = {
            if (onBack != null) {
                WontonPushedBar(title, onBack)
            } else {
                WontonRootBar(title) {
                    onAppearance?.let { WontonIconButton(WontonIcon.Palette, "外观", it) }
                }
            }
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
