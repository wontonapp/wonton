package app.wonton.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import app.wonton.client.ClientCore

/** 应用的根界面。平台入口（Android 的 Activity，以后的桌面窗口）只负责创建核心并调用它。 */
@Composable
fun WontonApp(core: ClientCore) {
    MaterialTheme {
        SessionListRoute(core)
    }
}
