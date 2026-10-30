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

/**
 * 应用的根界面。平台入口（Android 的 Activity，以后的桌面窗口）只负责创建核心并调用它。
 * `openWebOnReady` 只在开发时为真：网页入口和本地代理就绪后自动打开网页，不用点按也能测。
 */
@Composable
fun WontonApp(core: ClientCore, openWebOnReady: Boolean = false) {
    MaterialTheme {
        val updates = remember(core) { core.homeUpdates() }
        val initial = remember(core) { core.home() }
        val home by updates.collectAsState(initial = initial)
        var tab by rememberSaveable { mutableStateOf(if (openWebOnReady) 1 else 0) }
        var showingPage by rememberSaveable { mutableStateOf(false) }
        val url = home.webUrl
        val proxy = home.proxy
        LaunchedEffect(openWebOnReady, url, proxy) {
            if (openWebOnReady && url != null && proxy != null) showingPage = true
        }
        if (showingPage && url != null && proxy != null) {
            PluginPage(url = url, proxy = proxy, onClose = { showingPage = false })
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
                    else -> HomeRoute(core, home, onOpenWeb = { showingPage = true })
                }
            }
        }
    }
}

/** 核心起不来或开发时传入的连接信息被拒绝：如实显示原因，不进入主界面。 */
@Composable
fun StartupFailure(reason: String) {
    MaterialTheme {
        Box(Modifier.fillMaxSize().padding(24.dp)) {
            Text(reason, color = MaterialTheme.colorScheme.error)
        }
    }
}
