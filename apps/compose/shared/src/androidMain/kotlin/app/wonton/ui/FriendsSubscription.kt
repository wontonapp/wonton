package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.FriendsViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把好友页面的订阅包装成 [Flow]，取消和释放方式同其他核心订阅。 */
fun ClientCore.friendsUpdates(): Flow<FriendsViewModel> = flow {
    subscribeFriends().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
