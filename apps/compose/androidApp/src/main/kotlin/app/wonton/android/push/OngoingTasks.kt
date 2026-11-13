package app.wonton.android.push

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import app.wonton.android.IncomingRequests
import app.wonton.android.MainActivity
import app.wonton.android.R
import app.wonton.client.ClientCore
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.SettingsEntry
import app.wonton.ui.notificationChannelId

/**
 * 等主人确认的长任务（ADR-0101 §4）：核心标出持续等回答满一分钟的 Bot 卡片，壳只渲染进行中通知。
 * Android 16 起请求成为 Live Update；是否提升由系统决定，更早的版本显示普通的进行中通知。
 */
object OngoingTasks {
    private const val CHANNEL_ID = "tasks"
    /** 1 是消息，2 保持连接，3 端口映射，4 未读摘要，这里用 5。 */
    private const val NOTIFICATION_ID = 5

    fun sync(context: Context, core: ClientCore, view: ChatViewModel) {
        val pending = view.pendingConfirmations.filter { it.ongoing }
        if (!PushNotifications.allowed(context)) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (pending.isEmpty()) {
            manager.cancel(NOTIFICATION_ID)
            return
        }
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                context.getString(R.string.push_tasks_channel),
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
        val first = pending.first()
        val title = if (pending.size == 1) {
            context.getString(R.string.push_task_waiting_title, first.title)
        } else {
            context.getString(R.string.push_task_waiting_title_many, pending.size, first.title)
        }
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java)
                .setData(Uri.parse("wonton://ongoing-tasks"))
                .putExtra(IncomingRequests.EXTRA_SERVER, core.servers().servers.firstOrNull { it.current }?.id)
                .putExtra(PushBuilder.EXTRA_CARD, first.id)
                .putExtra(IncomingRequests.EXTRA_ROOM, first.roomId.orEmpty())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(title)
            .setContentIntent(open)
            .setCategory(Notification.CATEGORY_REMINDER)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setNumber(pending.size)
            .setTimeoutAfter((first.expiresAtMs.toLong() - System.currentTimeMillis()).coerceAtLeast(1))
            .setVisibility(Notification.VISIBILITY_PUBLIC)
        if (hiddenOnLockScreen(core)) {
            builder.setVisibility(Notification.VISIBILITY_PRIVATE)
                .setPublicVersion(
                    Notification.Builder(context, CHANNEL_ID)
                        .setSmallIcon(android.R.drawable.stat_notify_chat)
                        .setContentTitle(context.getString(R.string.push_hidden_confirmation))
                        .build(),
                )
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA) {
            builder.setRequestPromotedOngoing(true)
        }
        manager.notify(NOTIFICATION_ID, builder.build())
    }


    /** 锁屏上隐不隐藏由确认类别的开关定（ADR-0101 §5），取设置页的那一项。 */
    private fun hiddenOnLockScreen(core: ClientCore): Boolean =
        core.settings().sections
            .flatMap { section -> section.entries }
            .filterIsInstance<SettingsEntry.Notifications>()
            .flatMap { it.categories }
            .any { it.category == NotificationCategory.CONFIRMATION && it.lockScreenPrivate }
}
