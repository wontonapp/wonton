package app.wonton.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class PanelFeedLevelTest {
    @Test
    fun fullFallsBackWhenTheLastPageDisappears() {
        assertEquals(PanelLevel.PEEK, levelAfterFeed(PanelLevel.FULL, hadPages = true, hasPages = false))
    }

    @Test
    fun closedStaysClosedWhenTheFirstFeedIsEmpty() {
        assertEquals(PanelLevel.CLOSED, levelAfterFeed(PanelLevel.CLOSED, hadPages = false, hasPages = false))
    }

    @Test
    fun fullStaysWhenPagesRemainOrTheListWasAlreadyEmpty() {
        assertEquals(PanelLevel.FULL, levelAfterFeed(PanelLevel.FULL, hadPages = true, hasPages = true))
        assertEquals(PanelLevel.FULL, levelAfterFeed(PanelLevel.FULL, hadPages = false, hasPages = false))
        assertEquals(PanelLevel.PEEK, levelAfterFeed(PanelLevel.PEEK, hadPages = true, hasPages = false))
    }

    @Test
    fun emptyPanelStaysPeekWhenDraggedOutward() {
        assertEquals(PanelLevel.PEEK, levelAfterPeekDrag(pushes = true, closes = false, hasPages = false))
        assertEquals(PanelLevel.CLOSED, levelAfterPeekDrag(pushes = true, closes = true, hasPages = false))
    }

    @Test
    fun emptyAddPageStillReachesFullFromPeek() {
        assertEquals(PanelLevel.FULL, levelAfterPeekDrag(pushes = true, closes = false, hasPages = true))
        assertEquals(PanelLevel.PEEK, levelAfterPeekDrag(pushes = false, closes = false, hasPages = true))
    }
}
