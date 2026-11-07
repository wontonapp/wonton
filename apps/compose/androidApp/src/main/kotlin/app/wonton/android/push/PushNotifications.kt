package app.wonton.android.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.util.Log
import app.wonton.android.MainActivity
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushContent
import app.wonton.protocol.PushNotification
import app.wonton.ui.notificationChannelId
import app.wonton.ui.notificationTitle

/** 新消息的系统通知：同一个房间的通知合并成一条，显示最新的一条消息。确认卡片每张一条，答完或过期就撤下。 */
object PushNotifications {
    private const val TAG = "WontonPush"
    private const val NOTIFICATION_ID = 1

    /** 渠道要在发第一条通知前建好；建过再建不会改掉用户在系统设置里改过的选项。 */
    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.deleteNotificationChannel("messages")
        manager.createNotificationChannels(NotificationCategory.entries.map { category ->
            NotificationChannel(
                category.notificationChannelId, category.notificationTitle, NotificationManager.IMPORTANCE_HIGH,
            )
        })
    }

    fun show(
        context: Context,
        notification: PushNotification,
        tag: String = tag(notification.serverId, notification.roomId, notification.confirmation?.id),
    ) {
        if (context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            Log.w(TAG, "没有通知权限，这条推送弹不出来")
            return
        }
        createChannels(context)
        val channel = notification.category.notificationChannelId
        val title = notification.roomName ?: notification.sender
        val text = when (val content = notification.content) {
            is PushContent.Message ->
                if (notification.roomName != null) "${notification.sender}：${content.body}" else content.body
            is PushContent.Unavailable -> "新消息（内容暂时取不到：${content.message}）"
        }
        val card = notification.confirmation
        val open = PendingIntent.getActivity(
            context,
            // Intent data 包含完整目标，哈希碰撞也不会把两台服务器的通知合并。
            tag.hashCode(),
            Intent(context, MainActivity::class.java)
                .setData(Uri.parse("wonton://notification$tag"))
                .putExtra(MainActivity.EXTRA_SERVER, notification.serverId)
                .putExtra(MainActivity.EXTRA_ROOM, notification.roomId)
                .addFlags(
                    Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or
                        Intent.FLAG_ACTIVITY_SINGLE_TOP,
                ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        // 锁屏上隐藏敏感内容时显示这一版：不带发送者和正文。
        val hidden = Notification.Builder(context, channel)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle("Wonton")
            .setContentText("新消息")
            .build()
        val shown = Notification.Builder(context, channel)
            .setExtras(android.os.Bundle().apply {
                putString(MainActivity.EXTRA_SERVER, notification.serverId)
                putString("wonton_card", card?.id)
            })
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(hidden)
            .setContentIntent(open)
            .setAutoCancel(true)
            .setShowWhen(true)
        // App 被杀时收不到卡片的变化：到期这件事交给系统。
        if (card != null) {
            val left = card.expiresAtMs.toLong() - System.currentTimeMillis()
            shown.setTimeoutAfter(left.coerceAtLeast(1))
        }
        context.getSystemService(NotificationManager::class.java)
            .notify(tag, NOTIFICATION_ID, shown.build())
    }

    fun cancelCards(context: Context, serverId: String, cards: List<String>) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val settled = cards.toSet()
        manager.activeNotifications.filter { notification ->
            notification.notification.extras.getString(MainActivity.EXTRA_SERVER) == serverId &&
                notification.notification.extras.getString("wonton_card") in settled
        }.forEach { manager.cancel(it.tag, NOTIFICATION_ID) }
    }

    private fun tag(serverId: String?, roomId: String, cardId: String?): String =
        Uri.Builder().appendPath(roomId).appendPath(cardId.orEmpty())
            .appendQueryParameter("server", serverId).build().toString()
}
