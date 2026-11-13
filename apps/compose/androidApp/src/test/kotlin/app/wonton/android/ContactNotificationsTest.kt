package app.wonton.android

import android.app.Application
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Bundle
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.android.push.PushNotifications
import app.wonton.android.push.PushBuilder
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [36], application = Application::class)
class ContactNotificationsTest {
    @Test
    fun aFreshListRemovesOnlyHandledContactsOnItsOwnServer() {
        val context = ApplicationProvider.getApplicationContext<Application>()
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("test", "test", NotificationManager.IMPORTANCE_HIGH))
        manager.cancelAll()
        val active = "contact:app.example.agent:default:@alice:example.org"
        val handled = "contact:app.example.agent:default:@bob:example.org"
        val otherServer = "contact:app.example.agent:default:@carol:example.org"
        fun post(tag: String, server: String, card: String? = null) {
            val extras = Bundle().apply {
                putString(IncomingRequests.EXTRA_SERVER, server)
                card?.let { putString(PushBuilder.EXTRA_CARD, it) }
            }
            val notification = Notification.Builder(context, "test")
                .setSmallIcon(android.R.drawable.stat_notify_chat)
                .setExtras(extras)
                .build()
            manager.notify(tag, 1, notification)
        }
        try {
            post(active, "home")
            post(handled, "home")
            post(otherServer, "elsewhere")
            post("ordinary-confirmation", "home", "install-card")

            PushNotifications.reconcileContactRequests(context, "home", listOf(active))

            assertEquals(
                setOf(active, otherServer, "ordinary-confirmation"),
                manager.activeNotifications.map { it.tag }.toSet(),
            )
        } finally {
            manager.cancelAll()
            manager.deleteNotificationChannel("test")
        }
    }
}
