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
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import app.wonton.android.keepalive.KeepaliveService
import app.wonton.android.portmapping.PortMappingService
import app.wonton.android.portmapping.enabledPluginTitles
import app.wonton.android.push.PushNotifications
import app.wonton.android.push.PushRegistration
import app.wonton.client.ClientCore
import app.wonton.client.initLogging
import app.wonton.client.parseAppLink
import app.wonton.protocol.Action
import app.wonton.ui.AppLinkRequest
import app.wonton.ui.StartupFailure
import app.wonton.ui.RoomRequest
import app.wonton.ui.WontonApp
import app.wonton.ui.defaultDeviceName
import app.wonton.ui.homeUpdates
import app.wonton.ui.pluginsUpdates
import app.wonton.ui.send
import kotlinx.coroutines.flow.distinctUntilChangedBy
import kotlinx.coroutines.launch
import java.io.File

class MainActivity : AppCompatActivity() {
    /** 点通知要打开的房间。 */
    private var requestedRoom by mutableStateOf<RoomRequest?>(null)
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
            setContent { StartupFailure(getString(R.string.startup_core_failed, failure)) }
            return
        }
        // 开发时用 adb 传入配对码（dev.sh pair 输出的最后一行），模拟器上不用扫码；wonton_open_web 是插件网页的
        // 地址，本地代理就绪后自动打开：
        // adb shell am start -n app.wonton.android/.MainActivity \
        //   --es wonton_pairing_code '<码>' --es wonton_open_web <网址>
        val code = intent.getStringExtra("wonton_pairing_code")?.takeIf { debuggable }
        val rejected = code?.let { core.send(Action.PairHome(it, defaultDeviceName(this))) }
        if (rejected != null) {
            setContent { StartupFailure(getString(R.string.startup_code_rejected, rejected)) }
            return
        }
        val devWebUrl = if (debuggable) intent.getStringExtra("wonton_open_web") else null
        PushRegistration(this, core).start()
        keepConnectionService(core)
        portMappingService(core)
        requestRoom(intent)
        requestLink(intent)
        setContent {
            WontonApp(
                core,
                devWebUrl = devWebUrl,
                requestedRoom = requestedRoom,
                onRoomRequestHandled = { requestedRoom = null },
                requestedLink = requestedLink,
                onCardsSettled = { server, cards -> PushNotifications.cancelCards(this, server, cards) },
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
        val room = intent.getStringExtra(EXTRA_ROOM)
        val server = intent.getStringExtra(EXTRA_SERVER)
        if (room != null && server != null) {
            requestedLink = null
            requestedRoom = RoomRequest(server, room)
        }
        intent.removeExtra(EXTRA_ROOM)
        intent.removeExtra(EXTRA_SERVER)
    }

    /** “保持连接”的前台服务只看 `keep_connection`（ADR-0003 的“设计：保持连接”）：已配对、方式是保持连接时开，
     *  设置里关掉或没配对时停。变化只发生在界面上，Activity 活着的时候都接得住；服务比 Activity 活得久。 */
    private fun keepConnectionService(core: ClientCore) {
        lifecycleScope.launch {
            core.homeUpdates()
                .distinctUntilChangedBy { home -> home.keepConnection }
                .collect { home ->
                    if (home.keepConnection) {
                        ContextCompat.startForegroundService(
                            this@MainActivity,
                            Intent(this@MainActivity, KeepaliveService::class.java),
                        )
                    } else {
                        stopService(Intent(this@MainActivity, KeepaliveService::class.java))
                    }
                }
        }
    }

    /** 端口映射的前台服务只看 `port_mappings` 有没有开着映射的插件（ADR-0072）：有就带着插件名启动，
     *  全关了就停。两个前台服务互不启停，见 PortMappingService 的注释。 */
    private fun portMappingService(core: ClientCore) {
        lifecycleScope.launch {
            core.pluginsUpdates()
                .distinctUntilChangedBy { plugins -> plugins.portMappings.any { it.enabled } }
                .collect { plugins ->
                    if (plugins.portMappings.any { it.enabled }) {
                        ContextCompat.startForegroundService(
                            this@MainActivity,
                            Intent(this@MainActivity, PortMappingService::class.java)
                                .putStringArrayListExtra(
                                    PortMappingService.EXTRA_TITLES,
                                    ArrayList(enabledPluginTitles(plugins)),
                                ),
                        )
                    } else {
                        stopService(Intent(this@MainActivity, PortMappingService::class.java))
                    }
                }
        }
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
        const val EXTRA_SERVER = "wonton_open_server"
    }
}
