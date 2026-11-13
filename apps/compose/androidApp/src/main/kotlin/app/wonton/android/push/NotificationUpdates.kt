package app.wonton.android.push

import android.content.Context
import app.wonton.client.ClientCore
import app.wonton.ui.chatUpdates
import app.wonton.ui.settingsUpdates
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/** Activity 和保持连接服务共用的系统通知投影；业务计数与长等待资格都由核心给。 */
internal fun CoroutineScope.followSystemNotifications(context: Context, core: ClientCore): Job = launch {
    launch {
        core.subscribeUnread().use { subscription ->
            while (true) {
                val unread = subscription.next() ?: break
                UnreadBadge.refresh(context, unread.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt())
            }
        }
    }
    combine(core.chatUpdates(), core.settingsUpdates()) { chat, _ -> chat }.collectLatest { chat ->
        OngoingTasks.sync(context, core, chat)
        val server = core.servers().servers.firstOrNull { it.current }?.id
        if (server != null) PushNotifications.cancelCards(context, server, chat.settledConfirmations)
    }
}
