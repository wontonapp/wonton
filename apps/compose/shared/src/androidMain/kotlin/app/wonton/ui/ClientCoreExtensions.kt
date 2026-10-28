package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.SessionListViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/**
 * 把会话列表订阅包装成 [Flow]。
 *
 * 收集它的协程被取消时，UniFFI 会释放等待中的 Rust future；流结束时关闭订阅，释放 Rust 对象。
 */
fun ClientCore.sessionListUpdates(): Flow<SessionListViewModel> = flow {
    subscribeSessionList().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 把操作发给核心。被拒绝时返回核心给出的原因，供界面提示；成功时返回 null。 */
fun ClientCore.send(action: Action): String? =
    try {
        dispatch(action)
        null
    } catch (rejection: CoreException) {
        rejection.toString()
    }

/** 把和家里服务器连接的订阅包装成 [Flow]，取消和释放方式同 [sessionListUpdates]。 */
fun ClientCore.homeUpdates(): Flow<HomeViewModel> = flow {
    subscribeHome().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
