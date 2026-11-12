package app.wonton.ui

import androidx.compose.foundation.ScrollState
import androidx.compose.runtime.Composable
import app.wonton.protocol.Action
import app.wonton.protocol.FriendsViewModel
import app.wonton.protocol.PeerRow
import app.wonton.protocol.PeersState
import app.wonton.protocol.PeersViewModel

@Composable
internal fun PeerPageContent(
    peers: PeersViewModel,
    friends: FriendsViewModel,
    scroll: ScrollState,
    retry: () -> Unit,
    requestRemove: (PeerRow) -> Unit,
    send: (Action) -> Unit,
    showCode: () -> Unit,
    onScan: () -> Unit,
) {
    val state = peers.state
    val hasRows = friends.incoming.isNotEmpty() || peers.peers.isNotEmpty() ||
        peers.requests.isNotEmpty() || peers.rejected.isNotEmpty()
    when {
        hasRows -> PeerList(peers, friends.incoming, scroll, retry, requestRemove, send)
        state is PeersState.Failed ->
            ProblemView(state.problem(), retry.takeIf { state.error.retriesHelpfully() })
        state == PeersState.Loading -> LoadingView()
        else -> PeerEmpty(onShow = showCode, onScan = onScan)
    }
}
