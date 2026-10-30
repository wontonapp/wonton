package app.wonton.android.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.util.Log
import app.wonton.android.MainActivity
import app.wonton.protocol.PushContent
import app.wonton.protocol.PushNotification

/** 新消息的系统通知：同一个房间的通知合并成一条，显示最新的一条消息。 */
object PushNotifications {
    private const val TAG = "WontonPush"
    private const val CHANNEL = "messages"
    private const val NOTIFICATION_ID = 1

    /** 渠道要在发第一条通知前建好；建过再建不会改掉用户在系统设置里改过的选项。 */
    fun createChannel(context: Context) {
        val channel = NotificationChannel(CHANNEL, "新消息", NotificationManager.IMPORTANCE_HIGH).apply {
            description = "Bot 和家人发来的消息"
        }
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    fun show(context: Context, notification: PushNotification) {
        if (context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            Log.w(TAG, "没有通知权限，这条推送弹不出来")
            return
        }
        createChannel(context)
        val title = notification.roomName ?: notification.sender
        val text = when (val content = notification.content) {
            is PushContent.Message ->
                if (notification.roomName != null) "${notification.sender}：${content.body}" else content.body
            is PushContent.Unavailable -> "新消息（内容暂时取不到：${content.message}）"
        }
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE,
        )
        // 锁屏上隐藏敏感内容时显示这一版：不带发送者和正文。
        val hidden = Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle("Wonton")
            .setContentText("新消息")
            .build()
        val shown = Notification.Builder(context, CHANNEL)
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
            .build()
        context.getSystemService(NotificationManager::class.java)
            .notify(notification.roomId, NOTIFICATION_ID, shown)
    }
}
