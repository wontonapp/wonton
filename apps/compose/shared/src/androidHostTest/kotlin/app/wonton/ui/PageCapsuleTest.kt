package app.wonton.ui

import app.wonton.protocol.TabRef
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 只有公网网页留着胶囊；应用页的出口是左边缘右滑（ADR-0095）。 */
class PageCapsuleTest {
    @Test
    fun onlyPublicPagesKeepTheCapsule() {
        assertTrue(pageKeepsCapsule(TabRef.Url("https://example.com/")))
        assertFalse(pageKeepsCapsule(TabRef.Plugin("notes", null)))
        assertFalse(pageKeepsCapsule(TabRef.PluginPath("notes", "/a")))
    }
}
