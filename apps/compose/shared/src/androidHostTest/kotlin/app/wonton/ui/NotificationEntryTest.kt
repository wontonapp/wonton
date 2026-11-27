package app.wonton.ui

import org.junit.Assert.assertEquals
import org.junit.Test

/// “系统通知”入口的分支：允许了直接去系统页；没允许先弹授权框，弹完（允许、拒绝都）照样去系统页，
/// 拒绝不把人留在原地。
class NotificationEntryTest {
    private var asked = 0
    private var opened = 0
    private var answer: ((Boolean) -> Unit)? = null

    private fun tap(enabled: Boolean, canAsk: Boolean) {
        notificationEntry(
            enabled = enabled,
            canAsk = canAsk,
            ask = { onResult ->
                asked++
                answer = onResult
            },
            openSettings = { opened++ },
        )
    }

    @Test
    fun allowedOpensTheSettingsWithoutAsking() {
        tap(enabled = true, canAsk = true)
        assertEquals(0, asked)
        assertEquals(1, opened)
    }

    @Test
    fun grantingTheAskedPermissionStillOpensTheSettings() {
        tap(enabled = false, canAsk = true)
        assertEquals(1, asked)
        assertEquals(0, opened)
        answer?.invoke(true)
        assertEquals(1, opened)
    }

    @Test
    fun denyingTheAskedPermissionStillOpensTheSettings() {
        tap(enabled = false, canAsk = true)
        answer?.invoke(false)
        assertEquals(1, opened)
    }

    @Test
    fun systemWithoutAPermissionPromptOpensTheSettingsDirectly() {
        tap(enabled = false, canAsk = false)
        assertEquals(0, asked)
        assertEquals(1, opened)
    }
}
