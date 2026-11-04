package app.wonton.ui

import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.client.defaultAppearance
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.ui.design.WontonBarItem
import app.wonton.ui.design.WontonBottomBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.ChatFill
import app.wonton.ui.icons.Home
import app.wonton.ui.icons.HomeFill
import app.wonton.ui.icons.Store
import app.wonton.ui.icons.StoreFill
import app.wonton.ui.icons.WontonIcon

private const val CHAT = 0
private const val HOME = 1
private const val STORE = 2
private val tabs = listOf(
    WontonBarItem("聊天", WontonIcon.Chat, WontonIcon.ChatFill),
    WontonBarItem("家", WontonIcon.Home, WontonIcon.HomeFill),
    WontonBarItem("商店", WontonIcon.Store, WontonIcon.StoreFill),
)

/**
 * 应用的根界面。平台入口（Android 的 Activity，以后的桌面窗口）只负责创建核心并调用它。
 * `devWebUrl` 只在开发时给出：本地代理就绪后自动打开这个插件网页，不用点按也能测。
 */
@Composable
fun WontonApp(core: ClientCore, devWebUrl: String? = null) {
    val appearanceUpdates = remember(core) { core.appearanceUpdates() }
    val initialAppearance = remember(core) { core.appearance() }
    val appearance by appearanceUpdates.collectAsState(initial = initialAppearance)
    WontonTheme(appearance) {
        val updates = remember(core) { core.homeUpdates() }
        val initial = remember(core) { core.home() }
        val home by updates.collectAsState(initial = initial)
        val plugins = remember(core) { PluginsState(core) }
        val chat = rememberChatNavigation()
        // 还没配对时先到“家”的配对页；开发时要自动打开网页的也停在“家”。
        val startAtHome = initial.status is HomeStatus.NotConfigured || devWebUrl != null
        var tab by rememberSaveable { mutableIntStateOf(if (startAtHome) HOME else CHAT) }
        var page by remember { mutableStateOf<PluginEntry.Web?>(null) }
        var unavailable by remember { mutableStateOf<String?>(null) }
        // 主屏和商店里打开插件：Agent 插件切到聊天、打开和它的 Bot 的私聊；有网页的全屏打开。
        val open = remember(core) {
            { entry: PluginEntry ->
                when (entry) {
                    is PluginEntry.Chat -> {
                        tab = CHAT
                        unavailable = chat.openDirectChat(core, entry.botUserId)
                    }
                    is PluginEntry.Web ->
                        if (home.proxy == null) {
                            unavailable = "还没有连上家里的服务器：${home.status.text}"
                        } else {
                            page = entry
                        }
                }
            }
        }
        val homePlugins = remember(open) { PluginNavigation(open) }
        val storePlugins = remember(open) { PluginNavigation(open) }
        PluginsEffects(plugins)
        val proxy = home.proxy
        LaunchedEffect(devWebUrl, proxy) {
            if (devWebUrl != null && proxy != null) {
                page = PluginEntry.Web(devWebUrl, Uri.parse(devWebUrl).host.orEmpty())
            }
        }
        val shown = page
        if (shown != null && proxy != null) {
            BackHandler { page = null }
            PluginPage(shown.title, shown.url, proxy, onClose = { page = null })
            return@WontonTheme
        }
        MainTabs(
            tab = tab,
            onSelect = { tab = it },
            isRoomOpen = chat.openRoom != null,
        ) {
            when (tab) {
                CHAT -> ChatRoute(core, chat)
                HOME -> HomeRoute(core, home, plugins, homePlugins, appearance, onShowStore = { tab = STORE })
                else -> StoreRoute(plugins, storePlugins)
            }
        }
        unavailable?.let { reason -> Notice("打不开", reason, onDismiss = { unavailable = null }) }
        plugins.rejection?.let { reason -> Notice("操作被拒绝", reason, onDismiss = { plugins.rejection = null }) }
    }
}

/** 三个页面和底栏。聊天页和打字时让出底部的位置。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MainTabs(tab: Int, onSelect: (Int) -> Unit, isRoomOpen: Boolean, content: @Composable () -> Unit) {
    val showsBottomBar = !WindowInsets.isImeVisible && !(tab == CHAT && isRoomOpen)
    Scaffold(
        containerColor = WontonTheme.colors.canvas.toColor(),
        bottomBar = {
            if (showsBottomBar) WontonBottomBar(tabs, tab, onSelect)
        },
    ) { padding ->
        // 内层页面自己的标题栏和输入框不再重复让出系统栏和底部导航栏占掉的空间。
        Box(Modifier.padding(padding).consumeWindowInsets(padding).fillMaxSize()) {
            content()
        }
    }
}

/** 插件状态在主屏和商店之间共用，App 在前台时一直收集。 */
@Composable
private fun PluginsEffects(plugins: PluginsState) {
    LaunchedEffect(plugins) { plugins.observe() }
    // 名字和说明按主人的语言挑。系统语言变了时 Activity 重建，这里随之再告诉核心一次。
    val locales = LocalConfiguration.current.locales
    LaunchedEffect(plugins, locales) {
        plugins.refresh(Action.SetLanguages(locales.toLanguageTags().split(",")))
    }
}

@Composable
private fun Notice(title: String, reason: String, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = onDismiss) { Text("好") } },
        title = { Text(title) },
        text = { Text(reason) },
    )
}

/** 核心起不来或开发时传入的配对码被拒绝：如实显示原因，不进入主界面。 */
@Composable
fun StartupFailure(reason: String) {
    WontonTheme(defaultAppearance()) {
        Box(Modifier.fillMaxSize().padding(24.dp)) {
            Text(reason, color = MaterialTheme.colorScheme.error)
        }
    }
}
