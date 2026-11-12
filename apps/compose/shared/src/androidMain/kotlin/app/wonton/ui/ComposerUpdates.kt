package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.ComposerViewModel
import app.wonton.protocol.TimelineTarget
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 房间输入框的附件区和待发消息（ADR-0094）。订阅不了时流以 `CoreException` 结束；取消和释放同 [chatUpdates]。 */
fun ClientCore.composerUpdates(roomId: String): Flow<ComposerViewModel> = flow {
    subscribeComposer(TimelineTarget(roomId, null)).use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}
