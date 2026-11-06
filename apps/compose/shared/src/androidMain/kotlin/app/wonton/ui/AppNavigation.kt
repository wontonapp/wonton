package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.protocol.Action
import app.wonton.protocol.AppLink
import app.wonton.protocol.CoreException
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.OnboardingStep
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.TabRef

internal class AppNavigation(val tabs: TabsState, val chat: ChatNavigation) {
    var appsPage by mutableStateOf(HomePage.LAUNCHER)
    var settingsPage by mutableStateOf(SettingsSubpage.MAIN)
    var settingsOverlay by mutableStateOf(false)
    var tabMenu by mutableStateOf<TabRef?>(null)
    val plugins = PluginNavigation { entry ->
        when (entry) {
            is PluginEntry.Chat -> tabs.plugins.reject(
                chat.openDirectChat(tabs.core, entry.botUserId, tabs.viewModel.selected),
            )
            is PluginEntry.Web -> tabs.send(Action.SelectTab(entry.tab))
        }
    }

    fun openRoom(roomId: String) {
        settingsOverlay = false
        tabMenu = null
        tabs.send(Action.SelectTab(TabRef.Chat))
        chat.openWhenJoined(tabs.core, roomId)
    }

    fun finishOnboarding(peer: String?) {
        settingsOverlay = false
        chat.reset()
        tabs.send(Action.SelectTab(TabRef.Chat))
        if (peer != null) tabs.plugins.reject(chat.openDirectChat(tabs.core, peer))
    }

    fun openLink(link: AppLink, onboarding: OnboardingViewModel) {
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
                    if (peer == null) CoreException.NotReady("还没有默认 Agent，请先在引导中选择并安装。").problem
                    else chat.openDirectChat(tabs.core, peer, acceptInvite = true),
                )
            }
        }
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

    fun hidesBar(): Boolean = settingsOverlay || when (tabs.viewModel.selected) {
        TabRef.Chat -> chat.openRoom != null || chat.showingBackup || chat.startingChat
        TabRef.Apps -> appsPage != HomePage.LAUNCHER || plugins.detail != null || plugins.confirmation != null
        TabRef.Settings -> settingsPage != SettingsSubpage.MAIN
        is TabRef.Plugin -> false
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

@Composable
internal fun AppLinkEffect(request: AppLinkRequest?, navigation: AppNavigation) {
    LaunchedEffect(request) {
        if (request == null || request.handled) return@LaunchedEffect
        // 收到时仍未配对或已在引导中的导航直接忽略；冷启动只等待首次引导状态，不替主人完成引导。
        val core = navigation.tabs.core
        val paired = core.home().status !is HomeStatus.NotConfigured
        val initial = core.onboarding()
        if (!paired) {
            request.consume()
            return@LaunchedEffect
        }
        val onboarding = if (initial.step != null || initial.problem != null) initial else {
            core.subscribeOnboarding().use { subscription ->
                var view = initial
                while (view.step == null && view.problem == null) view = subscription.next() ?: break
                view
            }
        }
        if (onboarding.step == OnboardingStep.FINISHED) {
            if (pluginLinkReady(request.link, navigation.tabs.plugins) &&
                navigation.isReadyForLink()
            ) navigation.openLink(request.link, core.onboarding())
        }
        request.consume()
    }
}

private fun AppNavigation.isReadyForLink(): Boolean =
    tabs.core.home().status !is HomeStatus.NotConfigured && tabs.core.onboarding().step == OnboardingStep.FINISHED

private suspend fun pluginLinkReady(link: AppLink, plugins: PluginsState): Boolean {
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
        plugins.reject(loaded.problem?.problem ?: CoreException.NotReady("应用列表还没有准备好，请稍后重试。").problem)
    }
    return loaded.installed != null
}
