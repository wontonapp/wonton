package app.wonton.ui.widget

import android.content.Context
import androidx.glance.appwidget.updateAll
import app.wonton.client.ClientCore
import app.wonton.protocol.WidgetAccount
import app.wonton.protocol.WidgetSnapshot
import app.wonton.ui.removeRoomShortcuts
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.launch

/** 把系统入口快照的订阅包装成 [Flow]：每写下一份来一次。取消和释放方式同 `chatUpdates`。 */
fun ClientCore.widgetSnapshotUpdates(): Flow<WidgetSnapshot> = flow {
    subscribeWidgetSnapshot().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/**
 * 核心每写下一份快照（ADR-0100），让桌面上的三个小组件重画；没登录了就把桌面搜索里的会话一起撤掉。
 * 跟着进程走，不跟 Activity：保持连接的服务、推送服务在后台写快照时，小组件也跟着变。
 */
fun CoroutineScope.followWidgetSnapshots(context: Context, core: ClientCore) {
    val app = context.applicationContext
    launch {
        core.widgetSnapshotUpdates().collect { snapshot ->
            WidgetRevision.bump()
            RecentChatsWidget().updateAll(app)
            AgentStatusWidget().updateAll(app)
            AskAgentWidget().updateAll(app)
            if (snapshot.account != WidgetAccount.SIGNED_IN) removeRoomShortcuts(app)
        }
    }
}
