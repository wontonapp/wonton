package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import app.wonton.client.ClientCore
import app.wonton.protocol.ConfirmationsViewModel
import app.wonton.protocol.InboxViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

fun ClientCore.confirmationsUpdates(): Flow<ConfirmationsViewModel> = flow {
    subscribeConfirmations().use { subscription ->
        while (true) emit(subscription.next() ?: break)
    }
}

/** 收件箱（ADR-0119）：确认卡片是其中一类条目，铃铛的数字也从这里来。 */
fun ClientCore.inboxUpdates(): Flow<InboxViewModel> = flow {
    subscribeInbox().use { subscription ->
        while (true) emit(subscription.next() ?: break)
    }
}

/** Reconcile notifications even when a request ended while the app was stopped. */
@Composable
internal fun ContactNotificationsEffect(core: ClientCore, onActiveRequests: (List<String>) -> Unit) {
    LaunchedEffect(core) {
        core.confirmationsUpdates().collect { view ->
            if (!view.loading && view.problem == null) {
                onActiveRequests(view.contactRequests.map { request ->
                    "contact:${request.bot.pluginId}:${request.bot.bot}:${request.contact.userId}"
                })
            }
        }
    }
}
