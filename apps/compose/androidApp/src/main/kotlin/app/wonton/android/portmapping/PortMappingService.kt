package app.wonton.android.portmapping

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.ServiceCompat
import app.wonton.android.MainActivity
import app.wonton.android.R
import app.wonton.android.WontonApplication
import app.wonton.client.ClientCore
import app.wonton.protocol.PluginsViewModel
import app.wonton.ui.pluginsUpdates
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChangedBy
import kotlinx.coroutines.launch

/**
 * 端口映射的前台服务（ADR-0072）：有插件开着“映射到本机”时让进程和 iroh 隧道在后台不被挂起，
 * 主人在别的 App（Moonlight、RustDesk）里才连得上映射出来的端口。
 *
 * 和“保持连接”服务一样用 specialUse。两个服务不合并：启停条件来自两个不同的 ViewModel——保持连接是全局
 * 设置，映射是每个插件自己的开关；合并后通知内容要靠两个不相关开关的组合来挑，还得改 ADR-0003 里
 * “启停只看 keep_connection”的契约，而两个前台服务并存是 Android 允许的，各报各的原因。
 *
 * 启停只看 `PluginsViewModel.port_mappings` 有没有开着映射的插件：主界面在它变成真时启动本服务，假时停掉，
 * 这里自己不判断。通知标题跟着开着的映射走，插件列表变化时在这里刷新。
 */
class PortMappingService : Service() {
    /** 跟着插件列表刷新通知的协程都挂在这里；服务销毁时整个取消。 */
    private var scope: CoroutineScope? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val titles = initialTitles(intent)
        val core = (application as WontonApplication).core.getOrNull()
        if (titles.isEmpty() || core == null) {
            // 主界面只带非空列表启动；没有开着的映射就不该有常驻通知。
            stopSelf()
            return START_NOT_STICKY
        }
        // Android 14 起前台服务必须带着 manifest 里声明的类型进前台。
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            buildNotification(titles),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
        )
        if (scope == null) {
            val updates = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            scope = updates
            updates.launch { watchMappings(core) }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope?.cancel()
        scope = null
        super.onDestroy()
    }

    /** 渠道重要性低：常驻通知不响铃、不弹横幅。建过再建不会改掉用户在系统设置里改过的选项。 */
    private fun createChannel(context: Context) {
        val channel = NotificationChannel(
            CHANNEL_ID,
            context.getString(R.string.mapping_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        )
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    /** 插件列表变化时刷新通知标题；一个开着的映射都没有时什么都不做，主界面会停掉本服务。 */
    private suspend fun watchMappings(core: ClientCore) {
        val manager = getSystemService(NotificationManager::class.java)
        core.pluginsUpdates()
            .distinctUntilChangedBy { view -> enabledPluginTitles(view) }
            .collect { view ->
                val titles = enabledPluginTitles(view)
                if (titles.isNotEmpty()) manager.notify(NOTIFICATION_ID, buildNotification(titles))
            }
    }

    private fun buildNotification(titles: List<String>): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP,
            ),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val title = if (titles.size == 1) {
            getString(R.string.mapping_notification_title, titles[0])
        } else {
            getString(R.string.mapping_notification_title_multiple, titles.size)
        }
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.launch_glyph)
            .setContentTitle(title)
            .setContentIntent(open)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setOngoing(true)
            .build()
    }

    companion object {
        const val CHANNEL_ID = "port_mapping"
        const val NOTIFICATION_ID = 3

        /** 主界面启动服务时带的插件名，让第一版通知就有对的标题，不用等第一帧更新。 */
        const val EXTRA_TITLES = "wonton_mapping_titles"

        fun initialTitles(intent: Intent?): List<String> =
            intent?.getStringArrayListExtra(EXTRA_TITLES).orEmpty()
    }
}

/** 开着映射的插件名，按 ViewModel 里的顺序；已卸载的（列表里找不到了）用 id。 */
internal fun enabledPluginTitles(view: PluginsViewModel): List<String> {
    val titles = view.installed?.plugins?.associate { it.id to it.title }.orEmpty()
    return view.portMappings.filter { it.enabled }.map { titles[it.pluginId] ?: it.pluginId }
}
