package app.wonton.android

import android.Manifest
import android.os.Build
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import java.io.File
import java.io.RandomAccessFile
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/**
 * 壳的前后台契约（ADR-0032 的挂起门控）：失去可交互就该报后台，回到可交互就报前台。
 * 门控挂在 `AppBackgrounded` 上，关屏和系统对话框只会走到 `onPause`、到不了 `onStop`，
 * 所以用 `STARTED`（可见但不可交互）钉住它。用真 Activity 而不是假数据入口：`ui_fake` 的
 * dispatch 直接吞掉 Home 动作，看不到这两条日志。只看本次测试之后写的日志。
 */
@RunWith(AndroidJUnit4::class)
class LifecycleContractTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    // 这个用例只测前后台，不测权限：扫码框启动就请求相机，恢复的插件页和相册还可能请求麦克风、媒体。
    // 任一系统权限框在记 from 之前把 Activity 按下 PAUSED，随后的 moveToState(STARTED) 就会空转。
    // 所以把它们都先授好，让测试自己制造前后台切换。
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(
            Manifest.permission.POST_NOTIFICATIONS,
            Manifest.permission.CAMERA,
            Manifest.permission.RECORD_AUDIO,
            Manifest.permission.READ_MEDIA_IMAGES,
        )
    } else {
        GrantPermissionRule.grant(
            Manifest.permission.CAMERA,
            Manifest.permission.RECORD_AUDIO,
            Manifest.permission.READ_EXTERNAL_STORAGE,
        )
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun losingInteractivityReportsTheBackground() {
        val core = (compose.activity.application as WontonApplication).core
        check(core.isSuccess)
        val log = File(compose.activity.filesDir, "core.log")
        // 先把状态拉回 RESUMED 再记 from：启动时可能已经有别的系统框把 Activity 按到 PAUSED 并写过一行，
        // 那样随后那次 moveToState(STARTED) 就看不到新的切换了。
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        val from = log.length()

        compose.activityRule.scenario.moveToState(Lifecycle.State.STARTED)
        assertTrue("onPause 没有报后台", waitFor(log, from, "went to the background"))

        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        assertTrue("回前台没有报前台", waitFor(log, from, "came to the foreground"))
    }

    private fun waitFor(log: File, from: Long, needle: String): Boolean {
        repeat(50) {
            if (log.readTail(from).contains(needle)) return true
            Thread.sleep(100)
        }
        return false
    }

    private fun File.readTail(from: Long): String {
        if (!exists()) return ""
        RandomAccessFile(this, "r").use { file ->
            file.seek(from)
            val buffer = ByteArray((length() - from).toInt())
            file.readFully(buffer)
            return String(buffer)
        }
    }
}
