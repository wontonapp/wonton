package app.wonton.android

import android.content.Intent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.parseAppLink
import app.wonton.android.push.PushActionReceiver
import app.wonton.android.push.PushBuilder
import app.wonton.protocol.BotRef
import app.wonton.ui.AgentTextRequest
import app.wonton.ui.AppLinkRequest
import app.wonton.ui.RoomRequest

/** 外部进来的跳转：点通知的房间或联系人页、wonton:// 链接、分享的一句话。发给界面后由界面取走。 */
internal class IncomingRequests {
    var room by mutableStateOf<RoomRequest?>(null)
        private set
    var link by mutableStateOf<AppLinkRequest?>(null)
        private set
    var agentText by mutableStateOf<AgentTextRequest?>(null)
        private set

    /** extras 取走就删：屏幕旋转重建 Activity 时拿到的还是这个 Intent，不能再跳一次。 */
    fun take(intent: Intent) {
        takeContacts(intent)
        intent.getStringExtra(EXTRA_ROOM)?.let { room ->
            val server = intent.getStringExtra(EXTRA_SERVER)
            intent.removeExtra(EXTRA_ROOM)
            intent.removeExtra(EXTRA_SERVER)
            if (server != null) {
                // 新的跳转顶掉旧的：留下的只有最新这一个。
                link = null
                agentText = null
                this.room = RoomRequest(
                    server, room, cardId = intent.getStringExtra(PushBuilder.EXTRA_CARD),
                    threadRoot = intent.getStringExtra(PushActionReceiver.EXTRA_THREAD),
                )
                intent.removeExtra(PushBuilder.EXTRA_CARD)
                intent.removeExtra(PushActionReceiver.EXTRA_THREAD)
            }
        }
        if (intent.action == Intent.ACTION_VIEW) {
            intent.dataString?.let { url ->
                link = parseAppLink(url)?.let { parsed -> AppLinkRequest(parsed) { intent.data = null } }
            }
            if (link == null) intent.data = null
        }
        if (intent.action == Intent.ACTION_SEND && intent.type == "text/plain") {
            val text = intent.getStringExtra(Intent.EXTRA_TEXT)?.trim().orEmpty()
            intent.removeExtra(Intent.EXTRA_TEXT)
            // 空文字不是给云吞的；解析不开的 wonton:// 同样如实忽略。
            if (text.isNotEmpty()) {
                room = null
                link = null
                agentText = AgentTextRequest(text) { agentText = null }
            }
        }
    }

    /** 联系人请求的通知：去被请求的 Bot 的联系人页（ADR-0050），和点房间同一条路。 */
    private fun takeContacts(intent: Intent) {
        val plugin = intent.getStringExtra(EXTRA_CONTACTS_PLUGIN) ?: return
        val bot = intent.getStringExtra(EXTRA_CONTACTS_BOT)
        val server = intent.getStringExtra(EXTRA_SERVER)
        intent.removeExtra(EXTRA_CONTACTS_PLUGIN)
        intent.removeExtra(EXTRA_CONTACTS_BOT)
        intent.removeExtra(EXTRA_SERVER)
        if (server != null && bot != null) {
            link = null
            agentText = null
            room = RoomRequest(server, "", BotRef(pluginId = plugin, bot = bot))
        }
    }

    fun roomHandled() {
        room = null
    }

    companion object {
        const val EXTRA_ROOM = "wonton_open_room"
        const val EXTRA_SERVER = "wonton_open_server"
        const val EXTRA_CONTACTS_PLUGIN = "wonton_open_contacts_plugin"
        const val EXTRA_CONTACTS_BOT = "wonton_open_contacts_bot"
    }
}
