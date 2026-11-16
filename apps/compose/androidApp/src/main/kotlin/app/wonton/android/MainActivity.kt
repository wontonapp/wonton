package app.wonton.android

import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import app.wonton.android.keepalive.KeepaliveService
import app.wonton.android.portmapping.PortMappingService
import app.wonton.android.portmapping.enabledPluginTitles
import app.wonton.android.push.PushNotifications
import app.wonton.android.push.PushRegistration
import app.wonton.android.push.followSystemNotifications
import app.wonton.client.ClientCore
import app.wonton.client.initLogging
import app.wonton.protocol.Action
import app.wonton.protocol.HomeAction
import app.wonton.protocol.PluginsViewModel
import app.wonton.ui.LocalNotificationPermission
import app.wonton.ui.NotificationPermission
import app.wonton.ui.ShareIntake
import app.wonton.ui.StartupFailure
import app.wonton.ui.WontonApp
import app.wonton.ui.completeMatrixOidcCallback
import app.wonton.ui.defaultDeviceName
import app.wonton.ui.homeUpdates
import app.wonton.ui.pluginsUpdates
import app.wonton.ui.send
import kotlinx.coroutines.flow.distinctUntilChangedBy
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.File

internal const val UI_FAKE_EXTRA = "wonton_ui_fake"

open class MainActivity : AppCompatActivity() {
    private val requests = IncomingRequests()
    private val shareIntake = ShareIntake()
    private var shareInbox: ShareInbox? = null
    private var networkWatcher: NetworkWatcher? = null
    private lateinit var notificationPermission: NotificationPermission

    open override fun onCreate(savedInstanceState: Bundle?) {
        (application as WontonApplication).configureForLaunch(intent)
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
        val rejected = code?.let { core.send(Action.Home(HomeAction.PairHome(it, defaultDeviceName(this)))) }
        if (rejected != null) {
            setContent { StartupFailure(getString(R.string.startup_code_rejected, rejected)) }
            return
        }
        val devWebUrl = if (debuggable) intent.getStringExtra("wonton_open_web") else null
        notificationPermission = NotificationPermission(this)
        PushRegistration(this, core, notificationPermission).start()
        keepConnectionService(core)
        lifecycleScope.followSystemNotifications(this, core)
        portMappingService(core)
        refreshMappingNotificationOnGrant(core)
        shareInbox = ShareInbox(this, core, shareIntake)
        takeIntent(intent)
        setContent {
            CompositionLocalProvider(LocalNotificationPermission provides notificationPermission) {
                WontonApp(
                    core,
                    devWebUrl = devWebUrl,
                    requestedRoom = requests.room,
                    onRoomRequestHandled = requests::roomHandled,
                    requestedLink = requests.link,
                    shareIntake = shareIntake,
                    onCardsSettled = { server, cards -> PushNotifications.cancelCards(this, server, cards) },
                    onContactRequests = { server, active ->
                        PushNotifications.reconcileContactRequests(this, server, active)
                    },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        takeIntent(intent)
    }

    private fun takeIntent(intent: Intent) {
        if (takeMatrixCallback(intent)) return
        requests.take(intent)
        val inbox = shareInbox ?: return
        takeShare(intent)?.let { content -> lifecycleScope.launch { inbox.receive(content) } }
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
                        startPortMappingService(plugins)
                    } else {
                        stopService(Intent(this@MainActivity, PortMappingService::class.java))
                    }
                }
        }
    }

    private fun startPortMappingService(plugins: PluginsViewModel) {
        ContextCompat.startForegroundService(
            this@MainActivity,
            Intent(this@MainActivity, PortMappingService::class.java)
                .putStringArrayListExtra(
                    PortMappingService.EXTRA_TITLES,
                    ArrayList(enabledPluginTitles(plugins)),
                ),
        )
    }

    /** 没有权限时贴出的前台服务通知不会自己补进通知栏：权限变成有时把映射服务再启动一次，重贴通知。 */
    private fun refreshMappingNotificationOnGrant(core: ClientCore) {
        lifecycleScope.launch {
            var granted = notificationPermission.granted
            notificationPermission.grantedUpdates.collect { now ->
                if (now && !granted) {
                    val plugins = core.pluginsUpdates().first()
                    if (plugins.portMappings.any { it.enabled }) startPortMappingService(plugins)
                }
                granted = now
            }
        }
    }

    /** Matrix 登录的浏览器回调不是 App 链接：有进行中的登录就交给核心续上，没有就当过期的丢掉；两种都不当 App 链接。 */
    private fun takeMatrixCallback(intent: Intent): Boolean {
        val url = intent.dataString?.takeIf { intent.action == Intent.ACTION_VIEW } ?: return false
        val core = (application as WontonApplication).core.getOrNull()
        val taken = core?.completeMatrixOidcCallback(url) == true
        if (taken) intent.data = null
        return taken
    }

    override fun onResume() {
        super.onResume()
        // 回到前台：网络可能变了，让核心检查隧道并重连；通知权限也可能在系统设置里改过。
        (application as WontonApplication).core.getOrNull()?.send(Action.Home(HomeAction.AppForegrounded))
        if (::notificationPermission.isInitialized) notificationPermission.refresh()
    }

    override fun onStart() {
        super.onStart()
        val core = (application as WontonApplication).core.getOrNull() ?: return
        networkWatcher = NetworkWatcher(this) {
            core.send(Action.Home(HomeAction.NetworkChanged(it)))
        }.also { it.start() }
    }

    override fun onStop() {
        super.onStop()
        networkWatcher?.stop()
        networkWatcher = null
        (application as WontonApplication).core.getOrNull()?.send(Action.Home(HomeAction.AppBackgrounded))
    }
}
