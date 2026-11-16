package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import app.wonton.client.ClientCore
import app.wonton.protocol.ConfirmationsViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

fun ClientCore.confirmationsUpdates(): Flow<ConfirmationsViewModel> = flow {
    subscribeConfirmations().use { subscription ->
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
