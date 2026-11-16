package app.wonton.android.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import app.wonton.android.R
import app.wonton.android.IncomingRequests
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushNotification
import app.wonton.ui.notificationChannelId
import app.wonton.ui.notificationTitleRes

/*
 * 新消息的系统通知（ADR-0003、ADR-0101）：同一个房间的通知合并成一条，显示最新的一条消息；确认卡片每张一条，
 * 答完或过期就撤下。文字在 [PushTexts]，按钮和点开的意图在 [PushBuilder]，角标在 [UnreadBadge]。
 */

/** 所有通知和摘要同一个组：启动器把它收成一组，角标也挂在组上。 */
internal const val GROUP = "wonton"

/** 消息通知的 ID；带标签，一个房间一条。 */
internal const val NOTIFICATION_ID = 1

/** 未读摘要的 ID。已经占掉的号：2 保持连接、3 端口映射、5 等确认任务。 */
internal const val SUMMARY_ID = 4

object PushNotifications {
    private const val TAG = "WontonPush"

    /** 渠道要在发第一条通知前建好；建过再建不会改掉用户在系统设置里改过的选项。 */
    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.deleteNotificationChannel("messages")
        manager.createNotificationChannels(NotificationCategory.entries.map { category ->
            NotificationChannel(
                category.notificationChannelId, context.getString(category.notificationTitleRes),
                NotificationManager.IMPORTANCE_HIGH,
            )
        })
    }

    fun show(
        context: Context,
        notification: PushNotification,
        tag: String = PushBuilder.tag(notification),
        onlyAlertOnce: Boolean = false,
    ) {
        if (!PushNotifications.allowed(context)) {
            Log.w(TAG, "no notification permission, dropping this push")
            return
        }
        createChannels(context)
        val channel = notification.category.notificationChannelId
        val (title, text) = if (notification.hidden) {
            // App 锁着（ADR-0102）：通知栏里也不写发送者和正文。
            context.getString(R.string.app_name) to context.getString(R.string.push_hidden)
        } else {
            notification.contactRequest?.let { PushTexts.contact(context, it) }
                ?: PushTexts.message(context, notification)
        }
        val card = notification.confirmation
        val shown = Notification.Builder(context, channel)
            .setExtras(PushBuilder.extras(notification))
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setGroup(GROUP)
            .setContentIntent(PushBuilder.open(context, notification, tag))
            .setAutoCancel(true)
            .setShowWhen(true)
        // 锁屏上给不给看内容由主人按类别定（ADR-0101 §5）：关掉时锁屏只写一句占位。
        if (notification.lockScreenPrivate) {
            shown.setVisibility(Notification.VISIBILITY_PRIVATE)
                .setPublicVersion(PushTexts.hidden(context, notification.category))
        } else {
            shown.setVisibility(Notification.VISIBILITY_PUBLIC)
        }
        PushBuilder.actions(context, notification, tag, title).forEach(shown::addAction)
        if (onlyAlertOnce) {
            // 占位换成正文的回填重弹：同一个通知原地替换，别再响一遍。
            shown.setOnlyAlertOnce(true)
        }
        // App 被杀时收不到卡片的变化：到期这件事交给系统。
        if (card != null) {
            val left = card.expiresAtMs.toLong() - System.currentTimeMillis()
            shown.setTimeoutAfter(left.coerceAtLeast(1))
        }
        context.getSystemService(NotificationManager::class.java)
            .notify(tag, NOTIFICATION_ID, shown.build())
        UnreadBadge.refresh(context, notification.unreadTotal.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt())
    }

    fun cancelCards(context: Context, serverId: String, cards: List<String>) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val settled = cards.toSet()
        manager.activeNotifications.filter { notification ->
            notification.notification.extras.getString(IncomingRequests.EXTRA_SERVER) == serverId &&
                notification.notification.extras.getString(PushBuilder.EXTRA_CARD) in settled
        }.forEach { manager.cancel(it.tag, NOTIFICATION_ID) }
    }

    internal fun allowed(context: Context): Boolean =
        (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
            context.getSystemService(NotificationManager::class.java).areNotificationsEnabled()

    fun reconcileContactRequests(context: Context, serverId: String, activeRequests: List<String>) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val active = activeRequests.toSet()
        manager.activeNotifications.filter { notification ->
            val tag = notification.tag
            notification.notification.extras.getString(IncomingRequests.EXTRA_SERVER) == serverId &&
                tag?.startsWith("contact:") == true && tag !in active
        }.forEach { manager.cancel(it.tag, it.id) }
    }
}
