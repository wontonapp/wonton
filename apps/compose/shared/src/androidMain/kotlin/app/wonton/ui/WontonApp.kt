package app.wonton.ui

import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
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
import app.wonton.protocol.PendingConfirmation
import app.wonton.ui.design.WontonBarItem
import app.wonton.ui.design.WontonBottomBar
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Apps
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.ChatFill
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.SettingsFill
import app.wonton.ui.icons.WontonIcon

private const val CHAT = 0
private const val APPS = 1
private const val SETTINGS = 2
private val tabs = listOf(
    WontonBarItem("聊天", WontonIcon.Chat, WontonIcon.ChatFill),
    WontonBarItem("应用", WontonIcon.Apps, WontonIcon.AppsFill),
    WontonBarItem("设置", WontonIcon.Settings, WontonIcon.SettingsFill),
)

/**
 * 应用的根界面。平台入口（Android 的 Activity，以后的桌面窗口）只负责创建核心并调用它。
 * `devWebUrl` 只在开发时给出：本地代理就绪后自动打开这个插件网页，不用点按也能测。
 * `requestedRoom` 是点通知要打开的房间，打开后调 `onRoomRequestHandled`；`onCardsSettled` 收到不用再回答的卡片 ID，
 * 撤下它们的通知。
 */
@Composable
fun WontonApp(
    core: ClientCore,
    devWebUrl: String? = null,
    requestedRoom: String? = null,
    onRoomRequestHandled: () -> Unit = {},
    onCardsSettled: (List<String>) -> Unit = {},
) {
    val appearanceUpdates = remember(core) { core.appearanceUpdates() }
    val initialAppearance = remember(core) { core.appearance() }
    val appearance by appearanceUpdates.collectAsState(initial = initialAppearance)
    WontonTheme(appearance) {
        val updates = remember(core) { core.homeUpdates() }
        val initial = remember(core) { core.home() }
        val home by updates.collectAsState(initial = initial)
        val plugins = remember(core) { PluginsState(core) }
        val chat = rememberChatNavigation()
        // 提示条和撤通知不管在哪个页面都要，所以这里也收集一份。
        val chatUpdates = remember(core) { core.chatUpdates() }
        val initialChat = remember(core) { core.chat() }
        val chatView by chatUpdates.collectAsState(initial = initialChat)
        LaunchedEffect(chatView.settledConfirmations) { onCardsSettled(chatView.settledConfirmations) }
        // 还没配对时先到“应用”的配对页；开发时要自动打开网页的也停在“应用”。
        val startAtApps = initial.status is HomeStatus.NotConfigured || devWebUrl != null
        var tab by rememberSaveable { mutableIntStateOf(if (startAtApps) APPS else CHAT) }
        var page by remember { mutableStateOf<PluginEntry.Web?>(null) }
        var appsPage by rememberSaveable { mutableStateOf(HomePage.LAUNCHER) }
        var unavailable by remember { mutableStateOf<String?>(null) }
        // 应用页和应用商店里打开插件：Agent 插件切到聊天、打开和它的 Bot 的私聊；有网页的全屏打开。
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
        val openCardRoom = remember(core) {
            { roomId: String ->
                page = null
                tab = CHAT
                chat.openWhenJoined(core, roomId)
            }
        }
        LaunchedEffect(requestedRoom) {
            if (requestedRoom != null) {
                openCardRoom(requestedRoom)
                onRoomRequestHandled()
            }
        }
        val appsPlugins = remember(open) { PluginNavigation(open) }
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
            WithPendingCards(chatView.pendingConfirmations, openCardRoom) {
                PluginPage(shown.title, shown.url, proxy, onClose = { page = null })
            }
            return@WontonTheme
        }
        // 卡片所在的房间就在眼前时不用提示。
        val elsewhere = chatView.pendingConfirmations.filter { tab != CHAT || it.roomId != chat.openRoom }
        MainTabs(
            tab = tab,
            onSelect = { tab = it },
            hidesBottomBar = hidesBottomBar(tab, chat, appsPage, appsPlugins),
        ) {
            WithPendingCards(elsewhere, openCardRoom) {
                when (tab) {
                    CHAT -> ChatRoute(core, chat)
                    APPS -> HomeRoute(
                        core,
                        home,
                        plugins,
                        appsPlugins,
                        appsPage,
                        onPage = { appsPage = it },
                        pendingCards = chatView.pendingConfirmations,
                        onOpenCardRoom = openCardRoom,
                    )
                    else -> SettingsPage(core, appearance)
                }
            }
        }
        unavailable?.let { reason -> Notice("打不开", reason, onDismiss = { unavailable = null }) }
        plugins.rejection?.let { reason -> Notice("操作被拒绝", reason, onDismiss = { plugins.rejection = null }) }
    }
}

/** 三个页面和底栏。进到下一层的页面和打字时让出底部的位置。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MainTabs(tab: Int, onSelect: (Int) -> Unit, hidesBottomBar: Boolean, content: @Composable () -> Unit) {
    val showsBottomBar = !WindowInsets.isImeVisible && !hidesBottomBar
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

/** 有卡片在等时，在 [content] 上面放提示条，状态栏的位置由提示条让出，内层的标题栏不再重复让。 */
@Composable
private fun WithPendingCards(
    cards: List<PendingConfirmation>,
    onOpen: (String) -> Unit,
    content: @Composable () -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        if (cards.isNotEmpty()) PendingCardBanner(cards, onOpen)
        val below = if (cards.isNotEmpty()) Modifier.consumeWindowInsets(WindowInsets.statusBars) else Modifier
        Box(below.weight(1f).fillMaxSize()) { content() }
    }
}

/** 插件状态在应用页和应用商店之间共用，App 在前台时一直收集。 */
@Composable
private fun PluginsEffects(plugins: PluginsState) {
    LaunchedEffect(plugins) { plugins.observe() }
    // 名字和说明按主人的语言挑。系统语言变了时 Activity 重建，这里随之再告诉核心一次。
    val locales = LocalConfiguration.current.locales
    LaunchedEffect(plugins, locales) {
        plugins.refresh(Action.SetLanguages(locales.toLanguageTags().split(",")))
    }
}

/** 底栏只留在每个标签页的第一层。 */
private fun hidesBottomBar(tab: Int, chat: ChatNavigation, appsPage: HomePage, apps: PluginNavigation): Boolean =
    when (tab) {
        CHAT -> chat.openRoom != null
        APPS -> appsPage != HomePage.LAUNCHER || apps.detail != null || apps.confirmation != null
        else -> false
    }

@Composable
private fun Notice(title: String, reason: String, onDismiss: () -> Unit) {
    WontonDialog(title, reason, onDismiss)
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
