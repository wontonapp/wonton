package app.wonton.android

import android.app.Application
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.android.push.NotificationActions
import app.wonton.android.push.PushActionReceiver
import app.wonton.android.push.PushBuilder
import app.wonton.android.push.PushNotifications
import app.wonton.protocol.Answer
import app.wonton.protocol.CardChoice
import app.wonton.protocol.MessageSummary
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushConfirmation
import app.wonton.protocol.PushContent
import app.wonton.protocol.PushNotification
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [32], application = Application::class)
class PushActionsTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Test
    fun onlyRealChatMessagesOfferRemoteInput() {
        val message = message()
        val reply = PushBuilder.actions(context, message, "room", "Agent").single()
        assertNotNull(reply.remoteInputs)
        assertFalse(reply.actionIntent.isImmutable)
        val formCard = message.copy(category = NotificationCategory.CONFIRMATION,
            confirmation = PushConfirmation(id = "card", expiresAtMs = 10000uL, origin = null, choices = emptyList()))
        assertTrue(PushBuilder.actions(context, formCard, "card", "Agent").isEmpty())
        assertTrue(PushBuilder.actions(context, message.copy(roomId = ""), "room", "Agent").isEmpty())
        assertTrue(PushBuilder.actions(context,
            message.copy(category = NotificationCategory.PLUGIN_NOTICE), "room", "Agent").isEmpty())
        val actionable = formCard.copy(confirmation = formCard.confirmation!!.copy(
            choices = listOf(CardChoice(Answer.APPROVE, "Allow", null)),
        ))
        val choice = PushBuilder.actions(context, actionable, "card", "Agent").single()
        assertTrue(choice.actionIntent.isImmutable)
        assertTrue(choice.remoteInputs.isNullOrEmpty())
    }

    @Test
    fun failureRestoresCardAndKeepsPrivacyAndRoutingOnSeparateError() {
        PushNotifications.createChannels(context)
        val target = target("card-event").putExtra(PushBuilder.EXTRA_CARD, "card")
        val source = message().copy(eventId = "card-event",
            confirmation = PushConfirmation(id = "card", expiresAtMs = 10000uL, origin = null, choices = emptyList()))
        val public = Notification.Builder(context, "confirmation")
            .setSmallIcon(android.R.drawable.stat_notify_chat).setContentText("hidden").build()
        val original = Notification.Builder(context, "confirmation")
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentText("private card")
            .setExtras(PushBuilder.extras(source))
            .setContentIntent(PushBuilder.open(context, source, "room"))
            .setVisibility(Notification.VISIBILITY_PRIVATE).setPublicVersion(public).build()
        manager.notify("room", 1, original)

        NotificationActions.progress(context, target)
        val progress = manager.activeNotifications.single().notification
        assertEquals(Notification.VISIBILITY_PRIVATE, progress.visibility)
        assertEquals("hidden", progress.publicVersion.extras.getCharSequence(Notification.EXTRA_TEXT))
        NotificationActions.failure(context, target, "offline")
        val restored = manager.activeNotifications.single { it.id == 1 }.notification
        val error = manager.activeNotifications.single { it.id == 6 }.notification
        assertEquals("private card", restored.extras.getCharSequence(Notification.EXTRA_TEXT))
        assertEquals(original.contentIntent, error.contentIntent)
        assertEquals(Notification.VISIBILITY_PRIVATE, error.visibility)
        assertEquals("hidden", error.publicVersion.extras.getCharSequence(Notification.EXTRA_TEXT))
    }

    @Test
    fun acceptedOldReplyDoesNotDismissNewRoomMessage() {
        PushNotifications.createChannels(context)
        val latest = Notification.Builder(context, "message")
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setExtras(PushBuilder.extras(message().copy(eventId = "new-event"))).build()
        manager.notify("room", 1, latest)
        NotificationActions.accepted(context, target("old-event"))
        assertEquals("new-event", manager.activeNotifications.single().notification.extras
            .getString(PushActionReceiver.EXTRA_EVENT))
    }

    private fun target(event: String) = Intent()
        .putExtra(PushActionReceiver.EXTRA_TAG, "room")
        .putExtra(PushActionReceiver.EXTRA_EVENT, event)
        .putExtra(IncomingRequests.EXTRA_SERVER, "server")
        .putExtra(IncomingRequests.EXTRA_ROOM, "!room:example.org")

    private fun message() = PushNotification(
        serverId = "server", roomId = "!room:example.org", eventId = "event", sender = "Agent",
        roomName = null, content = PushContent.Message(MessageSummary.Text("hello")),
        category = NotificationCategory.MESSAGE, confirmation = null, contactRequest = null,
        threadRoot = null, lockScreenPrivate = false, unreadTotal = 1u,
    )
}
