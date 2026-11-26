package app.wonton.android

import android.app.Application
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.ui.restoredDraft
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/// 退出编辑态恢复草稿：核心对“进编辑前没打过字”回 `None`，恢复的是空正文，不是编辑的正文。
@RunWith(AndroidJUnit4::class)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
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
