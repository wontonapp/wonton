package app.wonton.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PageHistoryTest {
    private fun history(url: String, loaded: String = url): PageHistory = PageHistory().apply {
        started(url)
        finished(loaded)
    }

    @Test
    fun sameAddressIsSameDocument() {
        val page = history("http://app.test/app/")
        page.visited("http://app.test/app/#/album/2")
        assertTrue(page.isSameDocument("http://app.test/app/#/album/2"))
    }

    @Test
    fun redirectBackToThisDocumentIsSameDocument() {
        val page = history("http://app.test/", "http://app.test/app/")
        page.visited("http://app.test/app/#/album/2")
        assertTrue(page.isSameDocument("http://app.test/"))
    }

    @Test
    fun fragmentStepIsAnotherPage() {
        val page = history("http://app.test/app/#/album/2")
        page.visited("http://app.test/app/#/album/3")
        assertFalse(page.isSameDocument("http://app.test/app/#/album/2"))
    }

    @Test
    fun otherPathIsAnotherPage() {
        val page = history("http://app.test/app/a")
        page.visited("http://app.test/app/b")
        assertFalse(page.isSameDocument("http://app.test/app/a"))
    }
}
