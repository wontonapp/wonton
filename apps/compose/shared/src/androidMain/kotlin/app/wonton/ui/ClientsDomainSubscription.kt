package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.ClientsDomainViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把“HTTPS 和域名”设置页的订阅包装成 [Flow]，取消和释放方式同其他核心订阅。 */
fun ClientCore.clientsDomainUpdates(): Flow<ClientsDomainViewModel> = flow {
    subscribeClientsDomain().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
