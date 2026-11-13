package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.BotContactsState
import app.wonton.protocol.BotView
import app.wonton.protocol.ContactState
import app.wonton.protocol.PendingContactRequest
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme

internal class ConfirmationContactsState {
    var pending by mutableStateOf<PendingContactRequest?>(null)
    var remote by mutableStateOf<PendingContactRequest?>(null)
}

@Composable
internal fun ConfirmationContactEffects(plugins: PluginsState, state: ConfirmationContactsState) {
    val core = plugins.core
    LaunchedEffect(core) {
        core.botContactsUpdates().collect { view ->
            val pending = state.pending ?: return@collect
            if (view.bot != pending.bot) return@collect
            val requestEnded = when {
                view.actionFailure != null -> true
                view.state is BotContactsState.Failed -> true
                view.state == BotContactsState.Ready -> view.contacts.none {
                    it.userId == pending.contact.userId && it.state == ContactState.INCOMING
                }
                else -> false
            }
            if (requestEnded) state.pending = null
        }
    }
    state.remote?.let { request ->
        ApproveRemoteContactDialog(request.contact, { state.remote = null }) {
            state.remote = null
            state.pending = request
            val rejection = core.send(Action.ApproveBotContact(request.bot, request.contact.userId))
            if (rejection != null) state.pending = null
            plugins.reject(rejection)
        }
    }
}

@Composable
internal fun ConfirmationContactRow(
    request: PendingContactRequest,
    bots: List<BotView>,
    state: ConfirmationContactsState,
    onAction: (Action) -> Unit,
) {
    val bot = bots.firstOrNull { it.bot == request.bot }
    Column(
        Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(bot?.name ?: request.bot.bot, style = WontonTheme.typography.itemTitle)
        Text(request.bot.pluginId, style = WontonTheme.typography.supporting)
        Text(request.contact.userId, style = WontonTheme.typography.body)
        Text(stringResource(R.string.contacts_incoming_warning), style = WontonTheme.typography.supporting)
        if (state.pending == request) CircularProgressIndicator()
        if (state.pending == null) Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonSmallButton(stringResource(R.string.contacts_reject), {
                state.pending = request
                onAction(Action.RejectBotContact(request.bot, request.contact.userId))
            }, danger = true)
            WontonSmallButton(stringResource(R.string.contacts_approve), {
                if (request.contact.remote != null) state.remote = request else {
                    state.pending = request
                    onAction(Action.ApproveBotContact(request.bot, request.contact.userId))
                }
            })
        }
    }
}
