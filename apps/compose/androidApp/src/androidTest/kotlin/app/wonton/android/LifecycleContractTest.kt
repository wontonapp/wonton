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

    // 配对页的扫码框一启动就请求相机。没给的话系统权限框会把 Activity 按下 PAUSED，
    // 常常赶在测试记下 from 之前发生，这时 moveToState(STARTED) 因为状态已经是 STARTED 而空转。
    // 相机也先授好，让测试自己制造这次前后台切换。
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.CAMERA)
    } else {
        GrantPermissionRule.grant(Manifest.permission.CAMERA)
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
