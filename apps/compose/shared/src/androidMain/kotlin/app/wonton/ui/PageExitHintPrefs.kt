package app.wonton.ui

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * “怎么从整页应用回来”的引导看没看过，只记在本机：换设备、重装后再教一次不碍事，不值得进账号数据。
 * 进程里只有一份，界面读的是 Compose 状态：Debug 的启动参数和界面测试改了它，浮层跟着变。
 */
class PageExitHintPrefs private constructor(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var state by mutableStateOf(prefs.getBoolean(SEEN_KEY, false))

    var seen: Boolean
        get() = state
        set(value) {
            state = value
            prefs.edit().putBoolean(SEEN_KEY, value).apply()
        }

    companion object {
        private const val PREFS = "page_exit_hint"
        private const val SEEN_KEY = "seen"

        @Volatile
        private var shared: PageExitHintPrefs? = null

        fun of(context: Context): PageExitHintPrefs = shared ?: synchronized(this) {
            shared ?: PageExitHintPrefs(context.applicationContext).also { shared = it }
        }
    }
}
