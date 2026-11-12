package app.wonton.ui

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AppLink
import app.wonton.protocol.BotRef
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.TabRef

internal class AppNavigation(val tabs: TabsState, val chat: ChatNavigation, val context: Context) {
    var appsPage by mutableStateOf(HomePage.LAUNCHER)
    var settingsPage by mutableStateOf(SettingsSubpage.MAIN)
    // 配对提醒的链接（wonton://settings/peers）带着它来：其他服务器页滚回顶上的“请求”。
    var peersOpenRequests by mutableStateOf(false)
    var settingsOverlay by mutableStateOf(false)
    var tabMenu by mutableStateOf<TabRef?>(null)
    val plugins = PluginNavigation(
        open = { entry ->
            when (entry) {
                is PluginEntry.Chat -> tabs.plugins.reject(
                    chat.openDirectChat(tabs.core, entry.botUserId, tabs.viewModel.selected),
                )
                is PluginEntry.Web -> tabs.send(Action.SelectTab(entry.tab))
            }
        },
        checkInstalled = { checkInstalled() },
    )

    /** 结果不确定时去已安装列表看：关掉叠着的页面，回到主屏。 */
    fun checkInstalled() {
        appsPage = HomePage.LAUNCHER
        settingsOverlay = false
        plugins.detail = null
        plugins.confirmation = null
        plugins.changingVersion = null
        plugins.addingSource = false
        plugins.uninstalling = null
        tabs.send(Action.SelectTab(TabRef.Apps))
    }

    fun openRoom(roomId: String) {
        settingsOverlay = false
        tabMenu = null
        tabs.send(Action.SelectTab(TabRef.Chat))
        chat.openWhenJoined(tabs.core, roomId)
    }

    /** 联系人请求的通知点进来：这个 Bot 的详情页上打开联系人页。 */
    fun openContacts(bot: BotRef) {
        resetNavigation()
        detail(bot.pluginId)
        plugins.contacts = bot
    }

    fun finishOnboarding(peer: String?) {
        settingsOverlay = false
        chat.reset()
        tabs.send(Action.SelectTab(TabRef.Chat))
        if (peer != null) tabs.plugins.reject(chat.openDirectChat(tabs.core, peer))
    }

    /** 重置到主界面的导航状态：链接、分享进来时都从干净的一屏开始。 */
    private fun resetNavigation() {
        settingsOverlay = false
        chat.reset()
        tabs.pinRequest = null
        tabMenu = null
        appsPage = HomePage.LAUNCHER
        settingsPage = SettingsSubpage.MAIN
        plugins.detail = null
        plugins.confirmation = null
        plugins.uninstalling = null
        plugins.addingSource = false
        plugins.changingVersion = null
        plugins.contacts = null
    }

    fun openLink(link: AppLink, onboarding: OnboardingViewModel) {
        resetNavigation()
        when (link) {
            is AppLink.Tab -> if (link.tab == TabRef.Settings) settings() else tabs.send(Action.SelectTab(link.tab))
            AppLink.NewChat -> {
                tabs.send(Action.SelectTab(TabRef.Chat))
                chat.startingChat = true
            }
            AppLink.AgentChat -> {
                tabs.send(Action.SelectTab(TabRef.Chat))
                val peer = onboarding.primaryBotUserId
                tabs.plugins.reject(
                    if (peer == null) {
                        CoreException.NotReady(context.getString(R.string.problem_no_default_agent)).problem
                    }
                    else chat.openDirectChat(tabs.core, peer, acceptInvite = true),
                )
            }
            // 核心在“Wonton”私聊里的配对提醒带这个链接，直接到“请求”（ADR-0050）。
            AppLink.Peers -> {
                settings(SettingsSubpage.PEERS)
                peersOpenRequests = true
            }
        }
    }

    /**
     * 分享给云吞的一句话（iOS 快捷指令“问 Bot”同）：发给默认 Agent，打开和它的私聊。
     * 房间在核心建；本地列表还没到就按 await 机制等它出现，发起失败的原因弹在聊天页上。
     */
    fun askAgent(text: String, onboarding: OnboardingViewModel) {
        resetNavigation()
        tabs.send(Action.SelectTab(TabRef.Chat))
        tabs.core.send(Action.DismissChatProblem)
        val peer = onboarding.primaryBotUserId
        if (peer == null) {
            tabs.plugins.reject(CoreException.NotReady(context.getString(R.string.problem_no_default_agent)).problem)
            return
        }
        tabs.plugins.reject(
            tabs.core.send(Action.AskAgent(text)) ?: chat.openDirectChat(tabs.core, peer, acceptInvite = true),
        )
    }

    fun settings(page: SettingsSubpage = SettingsSubpage.MAIN) {
        settingsPage = page
        if (tabs.viewModel.items.any { it.tab == TabRef.Settings }) {
            tabs.send(Action.SelectTab(TabRef.Settings))
            settingsOverlay = false
        } else {
            // 核心的 SelectTab 不接受未固定的原生页，沿用设置的原生覆盖层。
            tabs.send(Action.CloseTabOverlay)
            settingsOverlay = true
        }
    }

    fun detail(id: String) {
        tabs.send(Action.SelectTab(TabRef.Apps))
        appsPage = HomePage.LAUNCHER
        plugins.detail = PluginRoute.Installed(id)
    }

    /** 侧边栏的入口：先收起侧边栏（WontonApp 做），再按目的地跳。 */
    fun sidebar(destination: SidebarDestination) {
        when (destination) {
            SidebarDestination.Chat -> selectChatList(startingChat = false)
            SidebarDestination.Store -> {
                tabs.send(Action.SelectTab(TabRef.Apps))
                appsPage = HomePage.STORE
            }
            SidebarDestination.AllApps -> {
                tabs.send(Action.SelectTab(TabRef.Apps))
                appsPage = HomePage.LAUNCHER
            }
            SidebarDestination.Settings -> settings()
            SidebarDestination.NewChat -> selectChatList(startingChat = true)
            is SidebarDestination.Plugin -> plugins.open(destination.entry)
        }
    }

    private fun selectChatList(startingChat: Boolean) {
        tabs.send(Action.SelectTab(TabRef.Chat))
        chat.openRoom = null
        chat.showingBackup = false
        chat.startingChat = startingChat
    }

    fun hidesBar(): Boolean = settingsOverlay || when (tabs.viewModel.selected) {
        TabRef.Chat -> chat.openRoom != null || chat.showingBackup || chat.startingChat
        TabRef.Apps -> appsPage != HomePage.LAUNCHER || plugins.detail != null || plugins.confirmation != null
        TabRef.Settings -> settingsPage != SettingsSubpage.MAIN
        is TabRef.Plugin, is TabRef.PluginPath, is TabRef.Bot, is TabRef.Url, is TabRef.SharedApp,
        is TabRef.UiResource -> false
    }
}

/** 每次系统 Intent 有自己的消费状态；重组或 Activity 重建不能重放已处理的导航。 */
class AppLinkRequest(val link: AppLink, val onHandled: () -> Unit) {
    internal var handled = false
        private set

    internal fun consume() {
        handled = true
        onHandled()
    }
}

/** 分享进来的一句话（ACTION_SEND）：发给默认 Agent（Action.AskAgent）。 */
class AgentTextRequest(val text: String, val onHandled: () -> Unit)

@Composable
internal fun AppLinkEffect(request: AppLinkRequest?, navigation: AppNavigation) {
    LaunchedEffect(request) {
        if (request == null || request.handled) return@LaunchedEffect
        // 收到时仍未配对或已在引导中的导航直接忽略；冷启动只等待首次引导状态，不替主人完成引导。
        val core = navigation.tabs.core
        val paired = core.home().status !is HomeStatus.NotConfigured
        if (!paired) {
            request.consume()
            return@LaunchedEffect
        }
        val onboarding = onboardingWhenKnown(core)
        if (onboarding.step == OnboardingStep.FINISHED) {
            val pluginReady = pluginLinkReady(request.link, navigation.tabs.plugins, navigation.context)
            if (pluginReady && navigation.isReadyForLink()) navigation.openLink(request.link, core.onboarding())
        }
        request.consume()
    }
}

private fun AppNavigation.isReadyForLink(): Boolean =
    tabs.core.home().status !is HomeStatus.NotConfigured && tabs.core.onboarding().step == OnboardingStep.FINISHED

/** 冷启动只等待首次引导状态，不替主人完成引导；等待结果给链接和分享共用。 */
private suspend fun onboardingWhenKnown(core: ClientCore): OnboardingViewModel {
    val initial = core.onboarding()
    return if (initial.step != null || initial.problem != null) initial else {
        core.subscribeOnboarding().use { subscription ->
            var view = initial
            while (view.step == null && view.problem == null) view = subscription.next() ?: break
            view
        }
    }
}

/** 分享给云吞的一句话：配对完成才发；没配对或引导没走完时如实忽略。 */
@Composable
internal fun AskAgentEffect(request: AgentTextRequest?, navigation: AppNavigation) {
    LaunchedEffect(request) {
        if (request == null) return@LaunchedEffect
        val core = navigation.tabs.core
        if (core.home().status is HomeStatus.NotConfigured) return@LaunchedEffect
        if (onboardingWhenKnown(core).step == OnboardingStep.FINISHED) {
            navigation.askAgent(request.text, core.onboarding())
        }
        request.onHandled()
    }
}

private suspend fun pluginLinkReady(link: AppLink, plugins: PluginsState, context: Context): Boolean {
    if (link !is AppLink.Tab || link.tab !is TabRef.Plugin) return true
    val core = plugins.core
    val initial = core.plugins()
    val loaded = if (initial.installed != null || initial.problem != null) initial else {
        core.subscribePlugins().use { subscription ->
            var view = initial
            while (view.installed == null && view.problem == null) view = subscription.next() ?: break
            view
        }
    }
    if (loaded.installed == null) {
        val reason = loaded.problem?.problem
            ?: CoreException.NotReady(context.getString(R.string.problem_apps_not_ready)).problem
        plugins.reject(reason)
    }
    return loaded.installed != null
}
