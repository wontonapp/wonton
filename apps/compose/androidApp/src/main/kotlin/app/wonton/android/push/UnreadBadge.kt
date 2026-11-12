package app.wonton.android.push

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import app.wonton.android.MainActivity
import app.wonton.android.R
import app.wonton.protocol.NotificationCategory
import app.wonton.ui.notificationChannelId

/**
 * 启动器角标（ADR-0101 §3）：数字是核心给的未读总数，不是通知条数。挂在组摘要上，多数启动器只把它画成圆点。
 */
object UnreadBadge {
    /** 核心给零就撤下摘要；打开 App 本身不改变这个数。 */
    fun refresh(context: Context, unread: Int) {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (unread > 0) {
            summary(context, unread)
        } else {
            manager.cancel(SUMMARY_ID)
        }
    }

    fun summary(context: Context, unread: Int) {
        if (!PushNotifications.allowed(context)) return
        PushNotifications.createChannels(context)
        val summary = Notification.Builder(context, NotificationCategory.MESSAGE.notificationChannelId)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(context.getString(R.string.app_name))
            .setGroup(GROUP)
            .setGroupSummary(true)
            .setNumber(unread)
            .setShowWhen(false)
            .setOnlyAlertOnce(true)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    0,
                    Intent(context, MainActivity::class.java).addFlags(
                        Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP,
                    ).setData(Uri.parse("wonton://unread-summary")),
                    PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .build()
        context.getSystemService(NotificationManager::class.java).notify(SUMMARY_ID, summary)
    }
}
