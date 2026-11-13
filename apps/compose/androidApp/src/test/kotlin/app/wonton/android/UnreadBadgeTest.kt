package app.wonton.android

import android.app.Application
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.android.push.UnreadBadge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [32], application = Application::class)
class UnreadBadgeTest {
    @Test
    fun absoluteUnreadCountStaysUntilCoreReportsRead() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val manager = context.getSystemService(NotificationManager::class.java)
        UnreadBadge.refresh(context, 7)
        val initial = manager.activeNotifications.single()
        assertEquals(7, initial.notification.number)
        org.robolectric.shadows.ShadowSystemClock.advanceBy(java.time.Duration.ofSeconds(1))
        UnreadBadge.refresh(context, 7)
        assertEquals(initial.postTime, manager.activeNotifications.single().postTime)
        UnreadBadge.refresh(context, 2)
        assertEquals(2, manager.activeNotifications.single().notification.number)
        UnreadBadge.refresh(context, 0)
        assertTrue(manager.activeNotifications.isEmpty())
    }
}
