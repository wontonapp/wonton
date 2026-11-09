package app.wonton.android.keepalive

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
import app.wonton.android.push.PushNotifications
import app.wonton.client.ClientCore
import app.wonton.ui.chatUpdates
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * “保持连接”的前台服务（ADR-0003 的“设计：保持连接”）：开着时进程和 iroh 隧道在后台不被挂起，Matrix 同步照常；
 * App 在后台时核心把按推送规则该通知的新消息收集进 `backgroundNotifications`，这里照着弹本地通知。
 *
 * 前台服务的类型用 specialUse：dataSync 在 Android 15 上每天只能跑 6 小时，connectedDevice 要求有配套外设，
 * 都不合适（ADR-0003 里写了 Google Play 上怎么声明）。和媒体播放的前台服务互不启停，通知各管各的。
 *
 * 启停只看 `HomeViewModel.keepConnection`：主界面在它变成真时启动本服务，假时停掉，这里自己不判断。
 */
class KeepaliveService : Service() {
    /** 收集后台通知的协程都挂在这里；服务销毁时整个取消。 */
    private var scope: CoroutineScope? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val core = (application as WontonApplication).core.getOrNull() ?: run {
            stopSelf()
            return START_NOT_STICKY
        }
        // Android 14 起前台服务必须带着 manifest 里声明的类型进前台。
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            buildNotification(),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
        )
        if (scope == null) {
            val updates = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            scope = updates
            updates.launch { showNewMessages(core) }
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
            context.getString(R.string.keepalive_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        )
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP,
            ),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.launch_glyph)
            .setContentTitle(getString(R.string.keepalive_notification_title))
            .setContentIntent(open)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setOngoing(true)
            .build()
    }

    /** 逐条弹后台同步到的新消息；同一个 event_id 用同一个通知标签，核心重复给也不会重复弹。 */
    private suspend fun showNewMessages(core: ClientCore) {
        val context = this
        val shown = mutableSetOf<String>()
        core.chatUpdates().collect { view ->
            view.backgroundNotifications.forEach { notification ->
                if (shown.add(notification.eventId)) {
                    PushNotifications.show(context, notification, tag = notification.eventId)
                }
            }
        }
    }

    companion object {
        const val CHANNEL_ID = "keepalive"
        const val NOTIFICATION_ID = 2
    }
}
