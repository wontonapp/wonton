package app.wonton.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class PluginBackTargetTest {
    private class Case(
        val name: String,
        val customView: Boolean,
        val history: HistoryStep?,
        val expected: PluginBackTarget,
    )

    private val cases = listOf(
        Case("custom_view_wins", true, HistoryStep(sameDocument = false), PluginBackTarget.CloseCustomView),
        Case("custom_view_wins_without_history", true, null, PluginBackTarget.CloseCustomView),
        Case("document_history_goes_back", false, HistoryStep(sameDocument = false), PluginBackTarget.History),
        Case("spa_same_document_closes_page", false, HistoryStep(sameDocument = true), PluginBackTarget.ClosePage),
        Case("no_history_closes_page", false, null, PluginBackTarget.ClosePage),
    )

    @Test
    fun targetByPress() {
        for (case in cases) {
            assertEquals(case.name, case.expected, pluginBackTarget(case.customView, case.history))
        }
    }

    @Test
    fun logUrlDropsQueryAndFragment() {
        assertEquals("http://app.test/app/", logUrl("http://app.test/app/?code=abc#/album/2"))
        assertEquals("http://app.test/app/", logUrl("http://app.test/app/#/album/2"))
    }
}
