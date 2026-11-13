package app.wonton.android.push

import android.app.Notification
import android.content.Context
import app.wonton.android.R
import app.wonton.protocol.ContactRequestPush
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushContent
import app.wonton.protocol.PushNotification
import app.wonton.ui.line
import app.wonton.ui.notificationChannelId

/**
 * 通知上的文字（ADR-0100）：正文和占位按平台语言写在这里，核心只给类别和原文。
 *
 * 私聊的标题是发送者，群里的正文前面带上发送者；确认卡片和联系人请求的标题由核心写好。
 */
object PushTexts {
    /** 消息和邀请。群里的正文前面带上发送者。 */
    fun message(context: Context, notification: PushNotification): Pair<String, String> {
        val sender = notification.sender ?: context.getString(R.string.push_unknown_sender)
        val text = when (val content = notification.content) {
            is PushContent.Message -> inRoom(context, notification, content.summary.line(context.resources))
            is PushContent.Invite -> inRoom(context, notification, context.getString(R.string.push_invite))
            is PushContent.Unavailable -> context.getString(R.string.push_unavailable)
        }
        return (notification.roomName ?: sender) to text
    }

    /** 联系人请求（ADR-0050）：核心只给原值，标题是发请求的一方，正文按有没有归属人写。 */
    fun contact(context: Context, request: ContactRequestPush): Pair<String, String> {
        val title = request.displayName?.takeIf { it.isNotBlank() } ?: request.userId
        val keeper = request.keeperDisplayName?.takeIf { it.isNotBlank() }
        val text = if (keeper != null) {
            context.getString(R.string.push_contact_request_keeper, keeper)
        } else {
            context.getString(R.string.push_contact_request)
        }
        return title to text
    }

    /** 锁屏上隐藏内容时的那一版：不带发送者和正文，只写一句类别。 */
    fun hidden(context: Context, category: NotificationCategory): Notification {
        val text = when (category) {
            NotificationCategory.CONFIRMATION -> R.string.push_hidden_confirmation
            NotificationCategory.PLUGIN_NOTICE -> R.string.push_hidden_notice
            NotificationCategory.MESSAGE -> R.string.push_hidden
        }
        return Notification.Builder(context, category.notificationChannelId)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(context.getString(R.string.app_name))
            .setContentText(context.getString(text))
            .build()
    }

    /** 群里的消息前面带上发送者；私聊的标题已经是发送者。 */
    private fun inRoom(context: Context, notification: PushNotification, text: String): String =
        if (notification.roomName == null) {
            text
        } else {
            val sender = notification.sender ?: context.getString(R.string.push_unknown_sender)
            context.getString(R.string.push_in_room, sender, text)
        }
}
