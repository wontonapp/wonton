package app.wonton.android.push

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Bundle
import app.wonton.android.IncomingRequests
import app.wonton.android.MainActivity
import app.wonton.android.R
import app.wonton.protocol.CardChoice
import app.wonton.protocol.PushContent
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushNotification
import app.wonton.ui.notificationChannelId

/**
 * 通知上的按钮和点开的意图（ADR-0101 §1）：消息给“回复”（RemoteInput），没有表单的确认卡片按核心给的选项
 * 给“允许/拒绝”。动作都回到进程里的 [PushActionReceiver]，由它拉起核心去发。
 */
object PushBuilder {
    /** 一张卡片最多给几个按钮：再多通知就摆不下了。 */
    private const val MAX_ACTIONS = 3

    /** 卡片 ID 跟着通知走，撤下已答的卡片时靠它找。 */
    const val EXTRA_CARD = "wonton_card"

    /** 通知按什么区分：确认卡片按卡片，联系人请求按发送者（核心给的房间为空），消息只按房间。 */
    fun tag(serverId: String?, roomId: String, cardId: String?): String =
        Uri.Builder().appendPath(roomId).appendPath(cardId.orEmpty())
            .appendQueryParameter("server", serverId).build().toString()

    fun tag(notification: PushNotification): String {
        val contact = notification.contactRequest ?: return tag(
            notification.serverId, notification.roomId, notification.confirmation?.id,
        )
        return Uri.Builder().appendPath("contact").appendPath(contact.bot.pluginId)
            .appendPath(contact.bot.bot).appendPath(contact.userId)
            .appendQueryParameter("server", notification.serverId).build().toString()
    }

    /** 撤下已答卡片时认的标记。 */
    fun extras(notification: PushNotification) = Bundle().apply {
        putString(IncomingRequests.EXTRA_SERVER, notification.serverId)
        putString(IncomingRequests.EXTRA_ROOM, notification.roomId)
        putString(PushBuilder.EXTRA_CARD, notification.confirmation?.id)
        putString(PushActionReceiver.EXTRA_EVENT, notification.eventId)
        putString(PushActionReceiver.EXTRA_THREAD, notification.threadRoot)
    }

    /** 点开去哪：联系人请求去被请求的 Bot 的联系人页，其余去房间。 */
    fun open(context: Context, notification: PushNotification, tag: String): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .putExtra(IncomingRequests.EXTRA_SERVER, notification.serverId)
            .putExtra(EXTRA_CARD, notification.confirmation?.id)
            .putExtra(PushActionReceiver.EXTRA_THREAD, notification.threadRoot)
            .addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or
                    Intent.FLAG_ACTIVITY_SINGLE_TOP,
            )
        val contact = notification.contactRequest
        if (contact != null) {
            intent.putExtra(IncomingRequests.EXTRA_CONTACTS_PLUGIN, contact.bot.pluginId)
                .putExtra(IncomingRequests.EXTRA_CONTACTS_BOT, contact.bot.bot)
        } else {
            intent.putExtra(IncomingRequests.EXTRA_ROOM, notification.roomId)
        }
        // data 带上完整目标，哈希碰撞也不会把两台服务器的通知合并。
        intent.data = Uri.parse("wonton://notification$tag")
        return PendingIntent.getActivity(
            context, tag.hashCode(), intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    /** 这条通知上的按钮：消息给回复，卡片给它的选项。 */
    fun actions(
        context: Context,
        notification: PushNotification,
        tag: String,
        title: String,
    ): List<Notification.Action> {
        val actions = mutableListOf<Notification.Action>()
        if (canReply(notification)) {
            actions += reply(context, notification, tag, title)
        }
        notification.confirmation?.choices.orEmpty().take(MAX_ACTIONS)
            .forEachIndexed { index, choice ->
                actions += answer(context, notification, tag, title, index, choice)
            }
        return actions
    }

    private fun canReply(notification: PushNotification): Boolean {
        if (notification.confirmation != null || notification.contactRequest != null) return false
        val target = notification.roomId.isNotBlank() && notification.eventId.isNotBlank()
        val message = notification.category == NotificationCategory.MESSAGE &&
            notification.content is PushContent.Message
        return target && message
    }

    private fun reply(
        context: Context,
        notification: PushNotification,
        tag: String,
        title: String,
    ): Notification.Action {
        val input = RemoteInput.Builder(PushActionReceiver.EXTRA_REPLY)
            .setLabel(context.getString(R.string.push_reply))
            .build()
        val intent = intent(context, notification, tag, title, PushActionReceiver.ACTION_REPLY)
        // 远程输入要系统往里填字，所以这个意图是 MUTABLE。
        intent.data = actionUri(notification, tag, "reply")
        return Notification.Action.Builder(
            Icon.createWithResource(context, android.R.drawable.ic_menu_send),
            context.getString(R.string.push_reply),
            PendingIntent.getBroadcast(
                context, tag.hashCode(), intent,
                PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            ),
        ).addRemoteInput(input).build()
    }

    private fun answer(
        context: Context,
        notification: PushNotification,
        tag: String,
        title: String,
        index: Int,
        choice: CardChoice,
    ): Notification.Action {
        val intent = intent(context, notification, tag, title, PushActionReceiver.ACTION_ANSWER)
            .putExtra(PushActionReceiver.EXTRA_ANSWER, choice.answer.name)
            .putExtra(PushActionReceiver.EXTRA_CHOICE_ID, choice.choiceId)
        intent.data = actionUri(notification, tag, "answer-$index")
        return Notification.Action.Builder(
            null as Icon?,
            choice.label,
            PendingIntent.getBroadcast(
                context, tag.hashCode() + index + 1, intent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            ),
        ).build()
    }

    private fun actionUri(notification: PushNotification, tag: String, action: String): Uri =
        Uri.Builder().scheme("wonton").authority("push-action")
            .appendPath(tag).appendPath(notification.eventId).appendPath(action).build()

    /** 动作要带上的目标：接收方靠它发回答/回复，也靠标题和类别在通知上写进展。 */
    private fun intent(
        context: Context,
        notification: PushNotification,
        tag: String,
        title: String,
        action: String,
    ): Intent = Intent(context, PushActionReceiver::class.java)
        .setAction(action)
        .putExtra(IncomingRequests.EXTRA_SERVER, notification.serverId)
        .putExtra(IncomingRequests.EXTRA_ROOM, notification.roomId)
        .putExtra(PushActionReceiver.EXTRA_THREAD, notification.threadRoot)
        .putExtra(PushActionReceiver.EXTRA_EVENT, notification.eventId)
        .putExtra(EXTRA_CARD, notification.confirmation?.id)
        .putExtra(PushActionReceiver.EXTRA_TAG, tag)
        .putExtra(PushActionReceiver.EXTRA_CHANNEL, notification.category.notificationChannelId)
        .putExtra(PushActionReceiver.EXTRA_TITLE, title)
        .putExtra(PushActionReceiver.EXTRA_PRIVATE, notification.lockScreenPrivate)
}
