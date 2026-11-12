package app.wonton.android.push

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import app.wonton.android.IncomingRequests
import app.wonton.android.MainActivity
import app.wonton.android.R
import app.wonton.protocol.NotificationCategory
import app.wonton.ui.notificationChannelId

/** Keep the card until accepted; progress and errors inherit privacy and the exact open target. */
object NotificationActions {
    private const val ORIGINAL = "wonton_action_original"
    private const val ERROR_ID = 6

    fun progress(context: Context, target: Intent) {
        val tag = target.getStringExtra(PushActionReceiver.EXTRA_TAG) ?: return
        val manager = context.getSystemService(NotificationManager::class.java)
        val existing = current(manager, tag)?.takeIf { matches(it, target) } ?: return
        val original = original(existing)
        val extras = Bundle(original.extras).apply { putParcelable(ORIGINAL, original) }
        val progress = Notification.Builder.recoverBuilder(context, original.clone())
            .setExtras(extras)
            .setActions()
            .setContentText(context.getString(R.string.push_sending))
            .setStyle(Notification.BigTextStyle().bigText(context.getString(R.string.push_sending)))
            .setProgress(0, 0, true)
            .setOnlyAlertOnce(true)
            .build()
        manager.notify(tag, NOTIFICATION_ID, progress)
    }

    fun failure(context: Context, target: Intent, reason: String) {
        if (!PushNotifications.allowed(context)) return
        PushNotifications.createChannels(context)
        val tag = target.getStringExtra(PushActionReceiver.EXTRA_TAG)
        val manager = context.getSystemService(NotificationManager::class.java)
        val existing = tag?.let { current(manager, it) }?.takeIf { matches(it, target) }
        val original = existing?.let(::original)
        if (original != null) manager.notify(tag, NOTIFICATION_ID, original)
        val category = NotificationCategory.entries.firstOrNull {
            it.notificationChannelId == target.getStringExtra(PushActionReceiver.EXTRA_CHANNEL)
        } ?: NotificationCategory.MESSAGE
        val text = context.getString(R.string.push_action_failed, reason)
        val error = Notification.Builder(context, category.notificationChannelId)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle(context.getString(R.string.app_name))
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setContentIntent(original?.contentIntent ?: open(context, target))
            // Missing metadata on an old action must never disclose its content on the lock screen.
            .setVisibility(original?.visibility ?: if (target.getBooleanExtra(PushActionReceiver.EXTRA_PRIVATE, true)) {
                Notification.VISIBILITY_PRIVATE
            } else {
                Notification.VISIBILITY_PUBLIC
            })
            .setPublicVersion(original?.publicVersion ?: PushTexts.hidden(context, category))
            .build()
        manager.notify("push-action-error:${tag.orEmpty()}", ERROR_ID, error)
    }

    fun accepted(context: Context, target: Intent) {
        val tag = target.getStringExtra(PushActionReceiver.EXTRA_TAG) ?: return
        val manager = context.getSystemService(NotificationManager::class.java)
        // A reply to event A must not dismiss event B which replaced the room notification meanwhile.
        if (current(manager, tag)?.let { matches(it, target) } == true) manager.cancel(tag, NOTIFICATION_ID)
        manager.cancel("push-action-error:$tag", ERROR_ID)
    }

    private fun current(manager: NotificationManager, tag: String): Notification? =
        manager.activeNotifications.firstOrNull { it.tag == tag && it.id == NOTIFICATION_ID }?.notification

    private fun matches(notification: Notification, target: Intent): Boolean =
        notification.extras.getString(PushActionReceiver.EXTRA_EVENT) ==
            target.getStringExtra(PushActionReceiver.EXTRA_EVENT) &&
            notification.extras.getString(PushBuilder.EXTRA_CARD) == target.getStringExtra(PushBuilder.EXTRA_CARD) &&
            notification.extras.getString(IncomingRequests.EXTRA_SERVER) ==
            target.getStringExtra(IncomingRequests.EXTRA_SERVER)

    @Suppress("DEPRECATION")
    private fun original(notification: Notification): Notification =
        notification.extras.getParcelable<Notification>(ORIGINAL) ?: notification

    private fun open(context: Context, target: Intent): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .putExtra(IncomingRequests.EXTRA_SERVER, target.getStringExtra(IncomingRequests.EXTRA_SERVER))
            .putExtra(IncomingRequests.EXTRA_ROOM, target.getStringExtra(IncomingRequests.EXTRA_ROOM))
            .putExtra(PushActionReceiver.EXTRA_THREAD, target.getStringExtra(PushActionReceiver.EXTRA_THREAD))
            .putExtra(PushBuilder.EXTRA_CARD, target.getStringExtra(PushBuilder.EXTRA_CARD))
            .addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP,
            )
        intent.data = Uri.Builder().scheme("wonton").authority("push-action-error")
            .appendPath(target.getStringExtra(PushActionReceiver.EXTRA_TAG).orEmpty())
            .appendPath(target.getStringExtra(PushActionReceiver.EXTRA_EVENT).orEmpty()).build()
        return PendingIntent.getActivity(context, 0, intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
}
