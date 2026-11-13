package app.wonton.android

import android.content.Intent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.parseAppLink
import app.wonton.android.push.PushBuilder
import app.wonton.protocol.BotRef
import app.wonton.ui.AppLinkRequest
import app.wonton.ui.RoomRequest

/** 外部进来的跳转：点通知的房间或联系人页、wonton:// 链接。发给界面后由界面取走；系统分享见 [ShareInbox]。 */
internal class IncomingRequests {
    var room by mutableStateOf<RoomRequest?>(null)
        private set
    var link by mutableStateOf<AppLinkRequest?>(null)
        private set

    /** extras 取走就删：屏幕旋转重建 Activity 时拿到的还是这个 Intent，不能再跳一次。 */
    fun take(intent: Intent) {
        takeContacts(intent)
        intent.getStringExtra(EXTRA_ROOM)?.let { room ->
            val server = intent.getStringExtra(EXTRA_SERVER)
            val sideChat = intent.getStringExtra(EXTRA_SIDE_CHAT)
            intent.removeExtra(EXTRA_ROOM)
            intent.removeExtra(EXTRA_SERVER)
            intent.removeExtra(EXTRA_SIDE_CHAT)
            if (server != null) {
                // 新的跳转顶掉旧的：留下的只有最新这一个。
                link = null
                this.room = RoomRequest(
                    server, room, cardId = intent.getStringExtra(PushBuilder.EXTRA_CARD),
                    sideChatRoot = sideChat,
                )
                intent.removeExtra(PushBuilder.EXTRA_CARD)
            }
        }
        if (intent.action == Intent.ACTION_VIEW) {
            intent.dataString?.let { url ->
                link = parseAppLink(url)?.let { parsed -> AppLinkRequest(parsed) { intent.data = null } }
            }
            if (link == null) intent.data = null
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
            room = RoomRequest(server, "", BotRef(pluginId = plugin, bot = bot))
        }
    }

    fun roomHandled() {
        room = null
    }

    companion object {
        const val EXTRA_ROOM = "wonton_open_room"
        const val EXTRA_SERVER = "wonton_open_server"
        const val EXTRA_SIDE_CHAT = "wonton_open_side_chat"
        const val EXTRA_CONTACTS_PLUGIN = "wonton_open_contacts_plugin"
        const val EXTRA_CONTACTS_BOT = "wonton_open_contacts_bot"
    }
}
