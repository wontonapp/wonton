package app.wonton.android

import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.android.push.PushNotifications
import app.wonton.android.push.PushRegistration
import app.wonton.client.initLogging
import app.wonton.client.parseAppLink
import app.wonton.protocol.Action
import app.wonton.ui.AppLinkRequest
import app.wonton.ui.StartupFailure
import app.wonton.ui.WontonApp
import app.wonton.ui.defaultDeviceName
import app.wonton.ui.send
import java.io.File

class MainActivity : AppCompatActivity() {
    /** 点通知要打开的房间。 */
    private var requestedRoom by mutableStateOf<String?>(null)
    private var requestedLink by mutableStateOf<AppLinkRequest?>(null)
    private var networkWatcher: NetworkWatcher? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        if (debuggable) {
            // 标准错误不进 logcat：核心日志写到数据目录，用 adb shell run-as app.wonton.android cat files/core.log 看。
            val filter = intent.getStringExtra("wonton_log") ?: "wonton_client=info,wonton_tunnel=info,warn"
            initLogging(filter, File(filesDir, "core.log").path)
        }
        val core = (application as WontonApplication).core.getOrElse { failure ->
            setContent { StartupFailure("核心启动失败：$failure") }
            return
        }
        // 开发时用 adb 传入配对码（dev.sh pair 输出的最后一行），模拟器上不用扫码；wonton_open_web 是插件网页的
        // 地址，本地代理就绪后自动打开：
        // adb shell am start -n app.wonton.android/.MainActivity \
        //   --es wonton_pairing_code '<码>' --es wonton_open_web <网址>
        val code = intent.getStringExtra("wonton_pairing_code")?.takeIf { debuggable }
        val rejected = code?.let { core.send(Action.PairHome(it, defaultDeviceName(this))) }
        if (rejected != null) {
            setContent { StartupFailure("开发时传入的配对码被拒绝：$rejected") }
            return
        }
        val devWebUrl = if (debuggable) intent.getStringExtra("wonton_open_web") else null
        PushRegistration(this, core).start()
        requestRoom(intent)
        requestLink(intent)
        setContent {
            WontonApp(
                core,
                devWebUrl = devWebUrl,
                requestedRoom = requestedRoom,
                onRoomRequestHandled = { requestedRoom = null },
                requestedLink = requestedLink,
                onCardsSettled = { cards -> PushNotifications.cancelCards(this, cards) },
            )
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        requestRoom(intent)
        requestLink(intent)
    }

    /** 取走就删：屏幕旋转重建 Activity 时拿到的还是这个 Intent，不能再跳一次。 */
    private fun requestRoom(intent: Intent) {
        intent.getStringExtra(EXTRA_ROOM)?.let {
            requestedLink = null
            requestedRoom = it
        }
        intent.removeExtra(EXTRA_ROOM)
    }

    private fun requestLink(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        val url = intent.dataString ?: return
        requestedLink = parseAppLink(url)?.let { link ->
            AppLinkRequest(link) { intent.data = null }
        }
        if (requestedLink == null) intent.data = null
    }

    override fun onResume() {
        super.onResume()
        // 回到前台：网络可能变了，让核心检查隧道并重连。
        (application as WontonApplication).core.getOrNull()?.send(Action.AppForegrounded)
    }

    override fun onStart() {
        super.onStart()
        val core = (application as WontonApplication).core.getOrNull() ?: return
        networkWatcher = NetworkWatcher(this) { core.send(Action.NetworkChanged(it)) }.also { it.start() }
    }

    override fun onStop() {
        super.onStop()
        networkWatcher?.stop()
        networkWatcher = null
        (application as WontonApplication).core.getOrNull()?.send(Action.AppBackgrounded)
    }

    companion object {
        const val EXTRA_ROOM = "wonton_open_room"
    }
}
