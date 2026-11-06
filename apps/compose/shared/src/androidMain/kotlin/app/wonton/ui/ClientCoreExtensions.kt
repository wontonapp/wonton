package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PluginsViewModel
import app.wonton.protocol.SettingsViewModel
import app.wonton.protocol.ServersViewModel
import app.wonton.protocol.TimelineUpdate
import app.wonton.protocol.TransfersViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/**
 * 把聊天状态的订阅包装成 [Flow]。
 *
 * 收集它的协程被取消时，UniFFI 会释放等待中的 Rust future；流结束时关闭订阅，释放 Rust 对象。
 */
fun ClientCore.chatUpdates(): Flow<ChatViewModel> = flow {
    subscribeChat().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/**
 * 批次的语义见核心的 `TimelineSubscription`。打不开时流以 [CoreException] 结束；取消和释放同 [chatUpdates]。
 */
fun ClientCore.timelineUpdates(roomId: String): Flow<TimelineUpdate> = flow {
    subscribeTimeline(roomId).use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把操作发给核心。被拒绝时返回要给人看的问题，供界面提示；成功时返回 null。 */
fun ClientCore.send(action: Action): Problem? =
    try {
        dispatch(action)
        null
    } catch (rejection: CoreException) {
        rejection.problem
    }

/** 把和家里服务器连接的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.homeUpdates(): Flow<HomeViewModel> = flow {
    subscribeHome().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把传输列表的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.transfersUpdates(): Flow<TransfersViewModel> = flow {
    subscribeTransfers().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把外观的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.appearanceUpdates(): Flow<AppearanceViewModel> = flow {
    subscribeAppearance().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把设置首页的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.settingsUpdates(): Flow<SettingsViewModel> = flow {
    subscribeSettings().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把插件页面的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.pluginsUpdates(): Flow<PluginsViewModel> = flow {
    subscribePlugins().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

fun ClientCore.serversUpdates(): Flow<ServersViewModel> = flow {
    subscribeServers().use { subscription ->
        while (true) emit(subscription.next() ?: break)
    }
}
