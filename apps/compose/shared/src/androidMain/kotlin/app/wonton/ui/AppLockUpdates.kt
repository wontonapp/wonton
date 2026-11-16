package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.AppLockViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把 App 锁的订阅包装成 [Flow]，取消和释放方式同 [ClientCore.chatUpdates]。 */
fun ClientCore.appLockUpdates(): Flow<AppLockViewModel> = flow {
    subscribeAppLock().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
