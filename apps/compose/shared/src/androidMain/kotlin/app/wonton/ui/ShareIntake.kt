package app.wonton.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.ShareInViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 分享进来、核心还没认领之前的状态（ADR-0097）：壳在复制收件目录时就出分享页的加载状态，读不了的项在这里报。 */
class ShareIntake {
    var copying by mutableStateOf(false)
    var problem by mutableStateOf<Problem?>(null)
}

/** 分享页的订阅包装成 [Flow]，取消和释放同 [chatUpdates]。 */
fun ClientCore.shareInUpdates(): Flow<ShareInViewModel> = flow {
    subscribeShare().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
