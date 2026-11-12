package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen

@Composable
internal fun ServerSettingsPage(
    plugins: PluginsState,
    home: HomeViewModel,
    chat: ChatViewModel,
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
        SettingsSubpage.INVITE -> WontonScreen(bar = {
            WontonPushedBar(stringResource(R.string.connection_add_device), back)
        }) {
            InviteScreen(core, home.invite, chat.encryption.verificationFlow, chat.encryption.keyHandoff)
        }
        SettingsSubpage.PAIRING -> PairingPage(stringResource(R.string.pairing_title_again), core, home.pairing, back)
        else -> error("Not a server settings page: $page")
    }
}
