package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.HomeStatus

/**
 * 应用的根界面。平台入口（Android 的 Activity，以后的桌面窗口）只负责创建核心并调用它。
 * `devWebUrl` 只在开发时给出：本地代理就绪后自动打开这个插件网页，不用点按也能测。
 */
@Composable
fun WontonApp(core: ClientCore, devWebUrl: String? = null) {
    MaterialTheme {
        val updates = remember(core) { core.homeUpdates() }
        val initial = remember(core) { core.home() }
        val home by updates.collectAsState(initial = initial)
        // 还没配对时先到“家”的配对页；开发时要自动打开网页的也停在“家”。
        val startAtHome = initial.status is HomeStatus.NotConfigured || devWebUrl != null
        var tab by rememberSaveable { mutableStateOf(if (startAtHome) 1 else 0) }
        var showingPage by rememberSaveable { mutableStateOf(false) }
        val proxy = home.proxy
        LaunchedEffect(devWebUrl, proxy) {
            if (devWebUrl != null && proxy != null) showingPage = true
        }
        if (showingPage && devWebUrl != null && proxy != null) {
            PluginPage(url = devWebUrl, proxy = proxy, onClose = { showingPage = false })
            return@MaterialTheme
        }
        Scaffold(
            bottomBar = {
                NavigationBar {
                    NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = {}, label = { Text("聊天") })
                    NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = {}, label = { Text("家") })
                }
            },
        ) { padding ->
            // 内层页面自己的标题栏和输入框不再重复让出系统栏和底部导航栏占掉的空间。
            Box(Modifier.padding(padding).consumeWindowInsets(padding).fillMaxSize()) {
                when (tab) {
                    0 -> ChatRoute(core)
                    else -> HomeRoute(core, home)
                }
            }
        }
    }
}

/** 核心起不来或开发时传入的配对码被拒绝：如实显示原因，不进入主界面。 */
@Composable
fun StartupFailure(reason: String) {
    MaterialTheme {
        Box(Modifier.fillMaxSize().padding(24.dp)) {
            Text(reason, color = MaterialTheme.colorScheme.error)
        }
    }
}
