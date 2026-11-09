package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
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

/** “应用”：没配对时是配对页，配上以后列出插件和应用商店。 */
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
    onConnection: () -> Unit,
    settingsUpgrade: Boolean,
    onSettings: (() -> Unit)? = null,
) {
    BackHandler(enabled = page != HomePage.LAUNCHER) { onPage(HomePage.LAUNCHER) }
    when {
        viewModel.status is HomeStatus.NotConfigured ->
            PairingPage(
                stringResource(R.string.pairing_title), core, viewModel.pairing, onBack = null,
                actions = if (onSettings != null) {
                    { SettingsToolbarButton(settingsUpgrade, onSettings) }
                } else null,
            )
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
                    onConnection = onConnection,
                    settingsUpgrade = settingsUpgrade,
                    onSettings = onSettings,
                )
            }
        }
    }
}

enum class HomePage { LAUNCHER, STORE }

@Composable
internal fun PairingPage(
    title: String,
    core: ClientCore,
    pairing: PairingStatus?,
    onBack: (() -> Unit)?,
    actions: (@Composable () -> Unit)? = null,
) {
    val back = onBack?.let { navigate ->
        {
            if (pairing is PairingStatus.Failed) core.send(Action.DismissPairingFailure)
            navigate()
        }
    }
    BackHandler(enabled = back != null) { back?.invoke() }
    WontonScreen(
        bar = {
            if (back != null) WontonPushedBar(title, back) else WontonRootBar(
                title,
                actions = if (actions != null) { { actions() } } else null,
            )
        },
    ) {
        PairingScreen(core, pairing, onPaired = { onBack?.invoke() }, onBack = onBack)
    }
}

/** 有新版本，主人没跳过，也没在升级。 */
fun CoreUpgradeViewModel.isPrompting() = offer != null && !offerSkipped
