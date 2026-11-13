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

internal class AppNavigation(val pages: PagesState, val chat: ChatNavigation, val context: Context) {
    // 应用网格这一层（全部应用、商店）：配对后的根页面是聊天，网格从侧边栏进，null 是没开。
    var appsPage by mutableStateOf<HomePage?>(null)
    var settingsPage by mutableStateOf(SettingsSubpage.MAIN)
    // 配对提醒的链接（wonton://settings/peers）带着它来：其他服务器页滚回顶上的“请求”。
    var peersOpenRequests by mutableStateOf(false)
    var settingsOverlay by mutableStateOf(false)
    var confirmationCard by mutableStateOf<String?>(null)

    // 网格这一层在页面底下留着，又盖着聊天根：从网格打开先收起它，不然关掉页面回到网格、私聊开在看不见的地方（iOS 同）。
    val plugins = PluginNavigation(
        open = { entry ->
            when (entry) {
                is PluginEntry.Chat -> {
                    val problem = chat.openDirectChat(pages.core, entry.botUserId, pages.current)
                    if (problem == null) closeApps()
                    pages.plugins.reject(problem)
                }
                is PluginEntry.Web -> {
                    closeApps()
                    pages.send(Action.OpenPage(entry.tab))
                }
            }
        },
        checkInstalled = { checkInstalled() },
    )

    /** 点通知进房间；通知来自旁聊时 [sideChatRoot] 是它的根，进那个旁聊（ADR-0093 §4）。 */
    fun openRoom(roomId: String, sideChatRoot: String? = null) {
        settingsOverlay = false
        closeApps()
        pages.send(Action.OpenPage(TabRef.Chat))
        chat.openWhenJoined(pages.core, roomId, sideChatRoot)
    }

    fun openConfirmations(cardId: String) {
        resetNavigation()
        pages.send(Action.OpenPage(TabRef.Chat))
        confirmationCard = cardId
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
        pages.send(Action.OpenPage(TabRef.Chat))
        if (peer != null) pages.plugins.reject(chat.openDirectChat(pages.core, peer))
    }

    /** 重置到主界面的导航状态：链接、分享进来时都从干净的一屏开始。 */
    private fun resetNavigation() {
        settingsOverlay = false
        confirmationCard = null
        chat.reset()
        closeApps()
        settingsPage = SettingsSubpage.MAIN
        plugins.confirmation = null
        plugins.uninstalling = null
        plugins.addingSource = false
        plugins.changingVersion = null
        plugins.contacts = null
    }

    fun openLink(link: AppLink, onboarding: OnboardingViewModel) {
        resetNavigation()
        when (link) {
            is AppLink.Tab -> pages.send(Action.OpenPage(link.tab))
            AppLink.Settings -> settings()
            AppLink.NewChat -> {
                pages.send(Action.OpenPage(TabRef.Chat))
                chat.startingChat = true
            }
            AppLink.AgentChat -> {
                pages.send(Action.OpenPage(TabRef.Chat))
                val peer = onboarding.primaryBotUserId
                pages.plugins.reject(
                    if (peer == null) {
                        CoreException.NotReady(context.getString(R.string.problem_no_default_agent)).problem
                    }
                    else chat.openDirectChat(pages.core, peer, acceptInvite = true),
                )
            }
            // 桌面小组件、桌面搜索点进来的会话（ADR-0100）。
            is AppLink.Room -> openRoom(link.roomId)
            // 核心在“Wonton”私聊里的配对提醒带这个链接，直接到“请求”（ADR-0050）。
            AppLink.Peers -> {
                settings(SettingsSubpage.PEERS)
                peersOpenRequests = true
            }
            // iOS 的分享扩展用它打开 App（ADR-0097）；Android 写好收件目录后直接认领，不经链接。
            is AppLink.Share -> pages.plugins.reject(rejectionOf { pages.core.claimShare(link.token) })
        }
    }

    fun settings(page: SettingsSubpage = SettingsSubpage.MAIN) {
        settingsPage = page
        settingsOverlay = true
    }

    /** 打开应用网格这一层（全部应用或商店），根页面回到聊天。 */
    fun openApps(page: HomePage) {
        appsPage = page
        pages.send(Action.OpenPage(TabRef.Chat))
    }

    /** 侧边栏的入口：先收起侧边栏（WontonApp 做），再按目的地跳。 */
    fun sidebar(destination: SidebarDestination) {
        when (destination) {
            SidebarDestination.Chat -> selectChatList(startingChat = false)
            SidebarDestination.Store -> openApps(HomePage.STORE)
            SidebarDestination.AllApps -> openApps(HomePage.LAUNCHER)
            SidebarDestination.Settings -> settings()
            SidebarDestination.NewChat -> selectChatList(startingChat = true)
            is SidebarDestination.Plugin -> plugins.open(destination.entry)
        }
    }

    private fun selectChatList(startingChat: Boolean) {
        settingsOverlay = false
        closeApps()
        pages.send(Action.OpenPage(TabRef.Chat))
        chat.openRoom = null
        chat.showingBackup = false
        chat.startingChat = startingChat
    }
}

/** 结果不确定时去已安装列表看：关掉叠着的页面，回到应用网格。 */
internal fun AppNavigation.checkInstalled() {
    appsPage = HomePage.LAUNCHER
    settingsOverlay = false
    plugins.detail = null
    plugins.confirmation = null
    plugins.changingVersion = null
    plugins.addingSource = false
    plugins.uninstalling = null
    pages.send(Action.OpenPage(TabRef.Apps))
}

/** 打开某个插件的详情页（在应用网格这一层上）。 */
internal fun AppNavigation.detail(id: String) {
    openApps(HomePage.LAUNCHER)
    plugins.detail = PluginRoute.Installed(id)
}

internal fun AppNavigation.closeApps() {
    appsPage = null
    plugins.detail = null
}

/** 栈顶这页归不归原生的层画（聊天根、应用网格；其余是整页的插件网页）。 */
internal fun TabRef.isNativeRoot(): Boolean = this == TabRef.Chat || this == TabRef.Apps

/** 原生层此刻画的是哪一块（重组不丢滚动位置用）。 */
internal fun AppNavigation.nativeKey(): String = when {
    settingsOverlay -> "settings/${settingsPage.name}"
    appsPage != null -> "apps/$appsPage"
    else -> "chat"
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

@Composable
internal fun AppLinkEffect(request: AppLinkRequest?, navigation: AppNavigation) {
    LaunchedEffect(request) {
        if (request == null || request.handled) return@LaunchedEffect
        // 收到时仍未配对或已在引导中的导航直接忽略；冷启动只等待首次引导状态，不替主人完成引导。
        val core = navigation.pages.core
        val paired = core.home().status !is HomeStatus.NotConfigured
        if (!paired) {
            request.consume()
            return@LaunchedEffect
        }
        val onboarding = onboardingWhenKnown(core)
        if (onboarding.step == OnboardingStep.FINISHED) {
            val pluginReady = pluginLinkReady(request.link, navigation.pages.plugins, navigation.context)
            if (pluginReady && navigation.isReadyForLink()) navigation.openLink(request.link, core.onboarding())
        }
        request.consume()
    }
}

private fun AppNavigation.isReadyForLink(): Boolean =
    pages.core.home().status !is HomeStatus.NotConfigured && pages.core.onboarding().step == OnboardingStep.FINISHED

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
