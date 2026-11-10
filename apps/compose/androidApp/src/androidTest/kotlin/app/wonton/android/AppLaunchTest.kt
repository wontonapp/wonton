package app.wonton.android

import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 设备上的启动测试骨架：先只确认进程起得来、页面没被系统收掉，之后的夜跑往回填真正的发起路径。
 * CI 现在只编它不跑（装一次 .so 太久），跑法见 `.github/workflows/README.md`。
 */
@RunWith(AndroidJUnit4::class)
@LargeTest
class AppLaunchTest {
    @get:Rule
    val activity = ActivityScenarioRule(MainActivity::class.java)

    @Test
    fun startsAndStaysOpen() {
        activity.scenario.onActivity { assertFalse(it.isFinishing) }
    }
}
