package app.wonton.android

import android.app.Application
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.BotRef
import app.wonton.ui.RoomRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** 点通知进来：联系人请求去被请求的 Bot 的联系人页（ADR-0050），取走后同一个 Intent 不再跳。 */
@RunWith(AndroidJUnit4::class)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class IncomingRequestsTest {
    @Test
    fun aContactRequestNotificationOpensThatBotsContacts() {
        val requests = IncomingRequests()
        val intent = Intent()
            .putExtra(IncomingRequests.EXTRA_SERVER, "server-1")
            .putExtra(IncomingRequests.EXTRA_CONTACTS_PLUGIN, "hermes")
            .putExtra(IncomingRequests.EXTRA_CONTACTS_BOT, "default")

        requests.take(intent)

        assertEquals(RoomRequest("server-1", "", BotRef(pluginId = "hermes", bot = "default")), requests.room)
        requests.roomHandled()
        requests.take(intent)
        assertNull(requests.room)
    }

    @Test
    fun aMessageNotificationStillOpensItsRoom() {
        val requests = IncomingRequests()

        requests.take(
            Intent().putExtra(IncomingRequests.EXTRA_SERVER, "server-1").putExtra(IncomingRequests.EXTRA_ROOM, "!r:h"),
        )

        assertEquals(RoomRequest("server-1", "!r:h"), requests.room)
    }
}
