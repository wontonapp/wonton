package app.wonton.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class KeepAlivePageLifecycleTest {
    @Test
    fun loadFinishingAfterCloseIsDeferredUntilReopen() {
        val lifecycle = KeepAlivePageLifecycle()

        lifecycle.setParked(true)
        assertFalse(lifecycle.loadFinished())
        assertEquals(KeepAlivePageLifecycle.ResumeAction.NONE, lifecycle.resume())

        lifecycle.setParked(false)
        assertEquals(
            KeepAlivePageLifecycle.ResumeAction.LOAD_SUCCEEDED,
            lifecycle.resume(),
        )
        assertEquals(KeepAlivePageLifecycle.ResumeAction.NONE, lifecycle.resume())
    }

    @Test
    fun loadFailureAfterCloseRequestsReloadOnReopen() {
        val lifecycle = KeepAlivePageLifecycle()

        lifecycle.setParked(true)
        assertFalse(lifecycle.loadFailed())
        lifecycle.setParked(false)

        assertEquals(KeepAlivePageLifecycle.ResumeAction.RELOAD, lifecycle.resume())
        assertEquals(KeepAlivePageLifecycle.ResumeAction.NONE, lifecycle.resume())
    }
}
