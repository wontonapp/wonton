package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.onStart

@Composable
fun WontonApp(
    core: ClientCore,
    devWebUrl: String? = null,
    requestedRoom: RoomRequest? = null,
    onRoomRequestHandled: () -> Unit = {},
    onCardsSettled: (String, List<String>) -> Unit = { _, _ -> },
    requestedLink: AppLinkRequest? = null,
    requestedAgentText: AgentTextRequest? = null,
) {
    val servers by remember(core) { core.serversUpdates() }.collectAsState(remember(core) { core.servers() })
    val appearance by remember(core) { core.appearanceUpdates() }.collectAsState(remember(core) { core.appearance() })
    var ready by remember(core) { mutableStateOf<RoomRequest?>(null) }
    var rejection by remember(core) { mutableStateOf<Problem?>(null) }
    val context = LocalContext.current
    val locales = LocalConfiguration.current.locales
    LaunchedEffect(core, requestedRoom, context, locales) {
        ready = null
        val request = requestedRoom ?: return@LaunchedEffect
        if (core.servers().servers.none { it.id == request.serverId && it.current }) {
            val problem = core.send(Action.SwitchServer(request.serverId))
            if (problem != null) {
                rejection = problem
                onRoomRequestHandled()
                return@LaunchedEffect
            }
        }
        val home = core.homeUpdates().onStart { emit(core.home()) }.first {
            it.status is HomeStatus.Connected || it.status is HomeStatus.Disconnected
        }
        val failure = home.status as? HomeStatus.Disconnected
        when {
            failure != null -> rejection = Problem(
                context.getString(R.string.servers_notification_unreachable), detail = failure.message,
            )
            core.servers().servers.any { it.id == request.serverId && it.current } -> ready = request
        }
        if (ready == null) onRoomRequestHandled()
    }
    val serverId = servers.servers.firstOrNull { it.current }?.id
    CompositionLocalProvider(LocalServers provides servers) {
        key(serverId, servers.adding) {
            ServerApp(
                core, devWebUrl, ready?.takeIf { it.serverId == serverId }, onRoomRequestHandled,
                onCardsSettled = { cards -> serverId?.let { onCardsSettled(it, cards) } },
                requestedLink = requestedLink,
                requestedAgentText = requestedAgentText,
            )
        }
    }
    WontonTheme(appearance) { rejection?.let { ProblemDialog(it) { rejection = null } } }
}
