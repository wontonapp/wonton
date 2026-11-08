package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.displayCutoutPadding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import app.wonton.ui.design.WontonEdgeToEdgeScaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.saveable.SaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.client.defaultAppearance
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.SettingsViewModel
import app.wonton.protocol.TabRef
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonBottomBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun ServerApp(
    core: ClientCore,
    devWebUrl: String?,
    requestedRoom: RoomRequest?,
    onRoomRequestHandled: () -> Unit,
    onCardsSettled: (List<String>) -> Unit,
    requestedLink: AppLinkRequest? = null,
) {
    val appearance by remember(core) { core.appearanceUpdates() }.collectAsState(remember(core) { core.appearance() })
    val settings by remember(core) { core.settingsUpdates() }.collectAsState(remember(core) { core.settings() })
    val home by remember(core) { core.homeUpdates() }.collectAsState(remember(core) { core.home() })
    val chatView by remember(core) { core.chatUpdates() }.collectAsState(remember(core) { core.chat() })
    val plugins = remember(core) { PluginsState(core) }
    val tabs = remember(core) { TabsState(plugins) }
    val draft = remember(core) { TabDraft() }
    val chat = rememberChatNavigation { tabs.send(Action.SelectTab(it)) }
    val context = LocalContext.current.applicationContext
    val navigation = remember(core, context) { AppNavigation(tabs, chat, context) }
    LaunchedEffect(tabs) { tabs.observe() }
    LaunchedEffect(chatView) { chat.onUpdate(chatView) }
    LaunchedEffect(chatView.settledConfirmations) { onCardsSettled(chatView.settledConfirmations) }
    LaunchedEffect(requestedRoom) {
        requestedRoom?.let { navigation.openRoom(it.roomId); onRoomRequestHandled() }
    }
    AppLinkEffect(requestedLink, navigation)
    AppShortcutsEffect(tabs)
    WontonTheme(appearance) {
        CompositionLocalProvider(LocalTabs provides tabs, LocalTabDraft provides draft) {
            PluginsEffects(plugins)
            OpenDevWebEffect(devWebUrl, plugins, tabs)
            if (LocalServers.current.adding) {
                AddServerPairing(plugins, home)
            } else OnboardingHost(plugins, home, chatView, navigation::finishOnboarding) {
                AppContent(navigation, home, settings, appearance, chatView)
                tabs.pinRequest?.let { TabPinSheet(tabs.viewModel, it) { tabs.pinRequest = null } }
            }
            DeviceVerificationDialogs(chatView, navigation, tabs.core)
            plugins.rejection?.let { ProblemDialog(it) { plugins.rejection = null } }
        }
    }
}


@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AppContent(
    nav: AppNavigation, home: HomeViewModel, settings: SettingsViewModel,
    appearance: AppearanceViewModel, chat: ChatViewModel,
) {
    val tabs = nav.tabs
    val view = tabs.viewModel
    val draft = LocalTabDraft.current
    val shown = view.overlay?.tab ?: view.selected
    val native = shown !is TabRef.Plugin || nav.settingsOverlay
    val fullscreen = !native && isFullscreen(tabs, shown)
    FullscreenSystemBars(fullscreen)
    val showsBar = showsBottomBar(nav, fullscreen, WindowInsets.isImeVisible)
    var reselected by remember { mutableStateOf<TabRef?>(null) }
    val nativeState = rememberSaveableStateHolder()
    val systemBottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val status = connectionBarStatus(home.notice, { tabs.send(Action.RetryConnection) }, nav::settings)
    val cards = chat.pendingConfirmations.filter {
        shown != TabRef.Chat || nav.settingsOverlay || it.roomId != nav.chat.openRoom
    }
    CompositionLocalProvider(LocalWontonBarStatus provides status) {
        WithPendingCards(cards, fullscreen, nav::openRoom) { bannerTop ->
            WontonEdgeToEdgeScaffold(
                containerColor = WontonTheme.colors.canvas.toColor(),
                contentWindowInsets = WindowInsets(0, 0, 0, 0),
                bottomBar = {
                    if (showsBar) WontonBottomBar(view, tabs.plugins, { selected ->
                        if (view.selected == selected) reselected = selected
                        tabs.send(Action.SelectTab(selected))
                    }, { nav.tabMenu = it })
                },
            ) { padding ->
                Box(Modifier.fillMaxSize()) {
                    PluginTabsHost(
                        tabs, home, appearance, if (showsBar) padding.calculateBottomPadding() else systemBottom,
                        active = shown.takeUnless { native }, reselected = reselected,
                        onReselected = { reselected = null }, onDetails = nav::detail,
                        onDraft = { text -> draft.text = text; tabs.send(Action.SelectTab(TabRef.Chat)) },
                        bannerTop = bannerTop,
                    )
                    if (native) Box(Modifier.fillMaxSize()) {
                        NativeContent(nav, home, settings, appearance, chat, nativeState)
                    }
                    OpeningChat(nav.chat)
                }
            }
        }
    }
    TabNotices(tabs) { nav.settings(SettingsSubpage.TABS) }
    view.items.firstOrNull { it.tab == nav.tabMenu }?.let { item ->
        TabBarMenu(item, tabs, { nav.settings(SettingsSubpage.TABS) }, { nav.tabMenu = null })
    }
}

private fun isFullscreen(tabs: TabsState, shown: TabRef): Boolean =
    (tabs.viewModel.overlay ?: tabs.viewModel.items.firstOrNull { it.tab == shown })?.page?.fullscreen == true

private fun showsBottomBar(nav: AppNavigation, fullscreen: Boolean, keyboard: Boolean): Boolean =
    !fullscreen && !keyboard && !nav.hidesBar() && nav.tabs.viewModel.overlay == null

@Composable
private fun OpeningChat(chat: ChatNavigation) {
    if (chat.isOpeningChat) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
    }
}

@Composable
private fun NativeContent(
    nav: AppNavigation, home: HomeViewModel, settings: SettingsViewModel, appearance: AppearanceViewModel,
    chat: ChatViewModel, holder: SaveableStateHolder,
) {
    holder.SaveableStateProvider(nav.tabs.viewModel.selected.toString()) {
        NativeTab(nav, home, settings, appearance, chat)
    }
}

@Composable
private fun NativeTab(
    nav: AppNavigation, home: HomeViewModel, settings: SettingsViewModel,
    appearance: AppearanceViewModel, chat: ChatViewModel,
) {
    val tabs = nav.tabs
    if (nav.settingsOverlay) BackHandler {
        if (nav.settingsPage == SettingsSubpage.MAIN) nav.settingsOverlay = false
        else nav.settingsPage = nav.settingsPage.parent()
    }
    when (if (nav.settingsOverlay) TabRef.Settings else tabs.viewModel.selected) {
        TabRef.Chat -> ChatRoute(tabs.core, nav.chat, tabs.plugins)
        TabRef.Apps -> HomeRoute(
            tabs.core, home, tabs.plugins, nav.plugins, nav.appsPage, { nav.appsPage = it },
            chat.pendingConfirmations, nav::openRoom,
            onConnection = { nav.settings(SettingsSubpage.SERVER) }, settingsUpgrade = settings.serverUpgrade,
            onSettings = { nav.settings() }.takeIf { tabs.viewModel.items.none { it.tab == TabRef.Settings } },
        )
        TabRef.Settings -> SettingsPage(
            tabs.plugins, settings, home, appearance, chat, nav.settingsPage,
            onClose = { nav.settingsOverlay = false }.takeIf { nav.settingsOverlay },
            peersOpenRequests = nav.peersOpenRequests,
            onPeersOpenRequestsHandled = { nav.peersOpenRequests = false },
        ) { nav.settingsPage = it }
        is TabRef.Plugin, is TabRef.SharedApp -> Unit
    }
}

@Composable
private fun OpenDevWebEffect(url: String?, plugins: PluginsState, tabs: TabsState) {
    val installed = plugins.viewModel.installed
    var handled by remember(url) { mutableStateOf(false) }
    LaunchedEffect(url, installed) {
        if (url != null && installed != null && !handled) {
            val plugin = installed.plugins.firstOrNull { it.webUrl == url }
            if (plugin != null) {
                handled = true
                plugin.tabChoices.firstOrNull()?.let { tabs.send(Action.SelectTab(it.tab)) }
            }
        }
    }
}

@Composable
private fun WithPendingCards(
    cards: List<PendingConfirmation>, fullscreen: Boolean, onOpen: (String) -> Unit,
    content: @Composable (bannerTop: Dp) -> Unit,
) {
    val density = LocalDensity.current
    var measured by remember { mutableStateOf(0.dp) }
    val banner = if (cards.isEmpty()) 0.dp else measured
    Box(Modifier.fillMaxSize()) {
        val below = if (cards.isNotEmpty() && !fullscreen) {
            Modifier.consumeWindowInsets(WindowInsets.statusBars)
        } else Modifier
        Box(below.padding(top = if (fullscreen) 0.dp else banner).fillMaxSize()) {
            content(if (fullscreen) banner else 0.dp)
        }
        // 提示条排在横幅下面：横幅自己让出状态栏，同层叠放会让提示条画进状态栏。
        if (cards.isNotEmpty()) Column(Modifier.onSizeChanged { measured = with(density) { it.height.toDp() } }
            .then(if (fullscreen) Modifier.displayCutoutPadding() else Modifier)) {
            PendingCardBanner(cards, onOpen)
            Box(Modifier.background(WontonTheme.colors.canvas.toColor())) {
                OnboardingTipNotice(app.wonton.protocol.OnboardingTip.CONFIRMATION_CARD)
            }
        }
    }
}

@Composable
private fun PluginsEffects(plugins: PluginsState) {
    LaunchedEffect(plugins) { plugins.observe() }
    val locales = LocalConfiguration.current.locales
    val shown = appLanguageTag()
    LaunchedEffect(plugins, locales, shown) { plugins.refresh(Action.SetLanguages(coreLanguageTags(shown, locales))) }
}

@Composable
fun StartupFailure(reason: String) {
    WontonTheme(defaultAppearance()) {
        Box(Modifier.fillMaxSize().padding(24.dp)) { Text(reason, color = MaterialTheme.colorScheme.error) }
    }
}
