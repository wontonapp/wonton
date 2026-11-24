package app.wonton.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.ui.restoredDraft
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/// 退出编辑态恢复草稿：核心对“进编辑前没打过字”回 `None`，恢复的是空正文，不是编辑的正文。
@RunWith(AndroidJUnit4::class)
class ComposerEditRestoreTest {
    @Test
    fun emptyDraftRestoresNothing() {
        val (body, reply) = restoredDraft(null, null)
        assertEquals("", body)
        assertNull(reply)
    }

    @Test
    fun savedDraftAndReplyComeBack() {
        val (body, reply) = restoredDraft("draft before", null)
        assertEquals("draft before", body)
        assertNull(reply)
    }
}
