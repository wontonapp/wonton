package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreUpgradeViewModel
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PairingStatus
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonScreen

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
    onSettings: (() -> Unit)? = null,
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
        page == HomePage.CONNECTION ->
            ConnectionPage(viewModel, plugins, onRetry = { core.send(Action.RetryConnection) }, onNavigate = onPage)
        page == HomePage.CONNECTION_DETAILS ->
            ConnectionDetailsPage(viewModel, onBack = { onPage(HomePage.CONNECTION) })
        page == HomePage.EGRESS -> EgressPage(plugins, onBack = { onPage(HomePage.CONNECTION) })
        page == HomePage.CORE_UPGRADE -> CoreUpgradePage(plugins, onBack = { onPage(HomePage.CONNECTION) })
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
                    onSettings = onSettings,
                )
            }
        }
    }
}

enum class HomePage { LAUNCHER, CONNECTION, CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, CORE_UPGRADE, STORE }

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

/** 有新版本，主人没跳过，也没在升级。 */
fun CoreUpgradeViewModel.isPrompting() = offer != null && !offerSkipped
