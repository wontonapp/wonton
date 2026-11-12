package app.wonton.ui

import androidx.compose.runtime.Composable
import app.wonton.protocol.Action
import app.wonton.protocol.HomeViewModel
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen

@Composable
internal fun ServerSettingsPage(
    plugins: PluginsState,
    home: HomeViewModel,
    page: SettingsSubpage,
    onPage: (SettingsSubpage) -> Unit,
) {
    val core = plugins.core
    val back = { onPage(SettingsSubpage.SERVER) }
    when (page) {
        SettingsSubpage.SERVER -> ConnectionPage(
            home, plugins, onRetry = { plugins.send(Action.RetryConnection) }, onNavigate = onPage,
        )
        SettingsSubpage.CONNECTION_DETAILS -> ConnectionDetailsPage(home, back)
        SettingsSubpage.EGRESS -> EgressPage(plugins, back)
        SettingsSubpage.REGISTRY_MIRROR -> RegistryMirrorPage(plugins, back)
        SettingsSubpage.CORE_UPGRADE -> CoreUpgradePage(plugins, back)
        SettingsSubpage.INVITE -> WontonScreen(bar = { WontonPushedBar("添加设备", back) }) {
            InviteScreen(core, home.invite)
        }
        SettingsSubpage.PAIRING -> PairingPage("重新配对", core, home.pairing, back)
        else -> error("Not a server settings page: $page")
    }
}
