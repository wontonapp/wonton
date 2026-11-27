package app.wonton.ui

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.displayCutoutPadding
import androidx.compose.foundation.layout.padding
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
import app.wonton.protocol.HomeAction
import app.wonton.protocol.BarEntry
import app.wonton.protocol.SettingsAction
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PagesViewModel
import app.wonton.protocol.SettingsViewModel
import app.wonton.protocol.TabRef
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.delay

@Composable
internal fun ServerApp(
    core: ClientCore,
    devWebUrl: String?,
    requestedRoom: RoomRequest?,
    onRoomRequestHandled: () -> Unit,
    onCardsSettled: (List<String>) -> Unit,
    requestedLink: AppLinkRequest? = null,
    shareIntake: ShareIntake = remember { ShareIntake() },
    onContactRequests: (List<String>) -> Unit = {},
) {
    val appearance by remember(core) { core.appearanceUpdates() }.collectAsState(remember(core) { core.appearance() })
    val settings by remember(core) { core.settingsUpdates() }.collectAsState(remember(core) { core.settings() })
    val home by remember(core) { core.homeUpdates() }.collectAsState(remember(core) { core.home() })
    val chatView by remember(core) { core.chatUpdates() }.collectAsState(remember(core) { core.chat() })
    val plugins = remember(core) { PluginsState(core) }
    val pages = remember(core) { PagesState(plugins) }
    val draft = remember(core) { TabDraft() }
    val chat = rememberChatNavigation { pages.send(Action.Pages(PageAction.OpenPage(it))) }
    val context = LocalContext.current.applicationContext
    val navigation = remember(core, context) { AppNavigation(pages, chat, context) }
    LaunchedEffect(pages) { pages.observe() }
    LaunchedEffect(chatView) { chat.onUpdate(chatView) }
    LaunchedEffect(chatView.settledConfirmations) { onCardsSettled(chatView.settledConfirmations) }
    ContactNotificationsEffect(core, onContactRequests)
    LaunchedEffect(requestedRoom) {
        requestedRoom?.let { request ->
            when {
                request.contacts != null -> navigation.openContacts(request.contacts)
                request.roomId.isBlank() && request.cardId != null -> navigation.openConfirmations(request.cardId)
                else -> navigation.openRoom(request.roomId, request.sideChatRoot)
            }
            onRoomRequestHandled()
        }
    }
    AppLinkEffect(requestedLink, navigation)
    AppShortcutsEffect(pages)
    WontonTheme(appearance) {
        val proxyRoute = pluginProxyRoute(home, plugins)
        val account = LocalServers.current.servers.firstOrNull { it.current }?.id
        CompositionLocalProvider(LocalPages provides pages, LocalTabDraft provides draft,
            LocalPluginProxy provides proxyRoute, LocalPluginAccount provides account) {
            PluginsEffects(plugins)
            OpenDevWebEffect(devWebUrl, plugins, pages)
            if (LocalServers.current.adding) {
                AddServerPairing(plugins, home)
            } else OnboardingHost(plugins, home, chatView, navigation::finishOnboarding) {
                AdaptiveChatLayout(
                    list = { ExpandedChatPane(navigation, chatView) },
                    enabled = home.status != HomeStatus.NotConfigured,
                ) {
                    AppContent(navigation, home, settings, appearance, chatView)
                }
                navigation.confirmationCard?.let { card ->
                    ConfirmationsScreen(core, card) { navigation.confirmationCard = null }
                }
            }
            TabMediaPermissions(pages)
            DeviceVerificationDialogs(chatView, navigation, pages.core)
            KeyHandoffHost(chatView, navigation, pages.core)
            ShareInHost(core, shareIntake, chatView.bots, plugins, navigation::openRoom)
            plugins.rejection?.let { ProblemDialog(it) { plugins.rejection = null } }
        }
    }
}

@Suppress("CyclomaticComplexMethod") // 整个主界面的层叠在一处，拆开要带一屏参数。
@Composable
private fun AppContent(
    nav: AppNavigation, home: HomeViewModel, settings: SettingsViewModel,
    appearance: AppearanceViewModel, chat: ChatViewModel,
) {
    val pages = nav.pages
    val view = pages.viewModel
    val draft = LocalTabDraft.current
    val shown = pages.current
    // 栈顶是插件网页时整页归网页；其余都是原生的（聊天根、应用网格）。设置盖在什么上面都算原生。
    val opened = view.stack.lastOrNull()
    val pluginShown = !pages.appsOpen && opened != null && !opened.tab.isNativeRoot()
    val native = !pluginShown || nav.settingsOverlay
    val expanded = LocalExpandedChat.current
    var showsSidebar by remember { mutableStateOf(false) }
    val nativeState = rememberSaveableStateHolder()
    val status = connectionBarStatus(home, { pages.send(Action.Home(HomeAction.RetryConnection)) }, nav::settings)
    val cards = chat.pendingConfirmations.filter {
        val listShown = shown == TabRef.Rooms && !nav.settingsOverlay && !pages.appsOpen &&
            nav.chat.showingConfirmations
        val roomShown = shown == TabRef.Rooms && !nav.settingsOverlay && !pages.appsOpen &&
            !nav.chat.showingBackup && !nav.chat.startingChat && it.roomId != null &&
            it.roomId == nav.chat.openRoom
        !listShown && !roomShown
    }
    val onDraft: (String) -> Unit = { text ->
        draft.text = text
        pages.select(BarEntry.Agent)
    }
    // 面板只在聊天页和房间里；拉开时左边缘归面板（A 块的手势用来关它）。
    val panelScope = nav.chat.openRoom?.let { PanelScope.Room(it) } ?: PanelScope.Main
    val panelActive = shown == TabRef.Rooms && !pluginShown && !nav.settingsOverlay &&
        !pages.appsOpen && !nav.chat.showingBackup && !nav.chat.startingChat && !nav.chat.showingConfirmations
    val panel = if (panelActive && (!expanded || nav.chat.openRoom != null)) {
        rememberChatPanelState(if (nav.chat.openRoom == null) PanelEdge.TOP else PanelEdge.BOTTOM, panelScope,
            nav.pages)
    } else null
    if (panel != null && panel.scope == PanelScope.Main) {
        SideEffect { panel.feed(view.panel, view.panelSelected) }
    }
    // 只在聊天的根页面响应左边缘右滑（开抽屉）：进了房间、页面或盖了别的层，左边缘是系统的返回手势。
    val edgeSwipe = !expanded && chatShown && nav.chat.openRoom == null
    BackRootExit(nav, view)
    CompositionLocalProvider(LocalWontonBarStatus provides status) {
        WithPendingCards(cards, pluginShown && !nav.settingsOverlay, nav::openCardTarget) { bannerTop ->
            val page: @Composable () -> Unit = {
                SidebarContainer(
                    isOpen = showsSidebar && !expanded,
                    edgeSwipe = edgeSwipe,
                    onSetOpen = { showsSidebar = it },
                    panel = { width, interactive ->
                        SidebarPanel(home, pages.plugins, pages, shown, width, interactive) { destination ->
                            showsSidebar = false
                            nav.sidebar(destination)
                        }
                    },
                ) {
                    WontonEdgeToEdgeScaffold(
                        containerColor = WontonTheme.colors.canvas.toColor(),
                        contentWindowInsets = WindowInsets(0, 0, 0, 0),
                        bottomBar = { },
                    ) {
                        Box(Modifier.fillMaxSize()) {
                            PluginPagesHost(
                                pages, home, appearance,
                                active = opened?.tab?.takeIf { !native },
                                onDetails = nav::detail, onDraft = onDraft, bannerTop = bannerTop,
                            )
                            if (native) Box(Modifier.fillMaxSize()) {
                                CompositionLocalProvider(LocalWontonOpenSidebar provides
                                    if (expanded) null else ({ showsSidebar = true })) {
                                    NativeContent(nav, home, settings, appearance, chat, nativeState)
                                }
                                OpeningChat(nav.chat)
                            }
                        }
                    }
                }
            }
            page()
        }
    }
    BackHandler(enabled = showsSidebar && !expanded) { showsSidebar = false }
    TabNotices(pages)
}

/** 到了根页面以后，“再按一次退出”认多久。 */
private const val BACK_EXIT_MS = 2_000L

/**
 * 谁都没接住的返回：页面自己的返回（视频全屏、网页历史、关页）排在前面；这里只在它的处理器没装上时
 * 兜底关页。到了根页面才谈退出：第一下提示，第二下在 [BACK_EXIT_MS] 内退出。
 * 关页和退出之间留这一下，刚关掉页面又习惯性补按返回时，不会把云吞直接摔回桌面。
 */
@Composable
private fun BackRootExit(nav: AppNavigation, view: PagesViewModel) {
    val pages = nav.pages
    val activity = LocalActivity.current
    val context = LocalContext.current
    var armed by remember { mutableStateOf(false) }
    val showed = pages.current
    // 页面栈、盖子或网格一变，上一次的“再按一次”作废：刚落到根页面的那一下先不算退出。
    LaunchedEffect(showed, pages.appsOpen, nav.settingsOverlay) { armed = false }
    LaunchedEffect(armed) {
        if (armed) {
            delay(BACK_EXIT_MS)
            armed = false
        }
    }
    BackHandler {
        when {
            pages.appsOpen -> nav.closeApps()
            view.stack.isNotEmpty() && !nav.settingsOverlay ->
                pages.send(Action.Pages(PageAction.ClosePage))
            armed -> activity?.finish()
            else -> {
                armed = true
                Toast.makeText(context, R.string.common_back_exit_hint, Toast.LENGTH_SHORT).show()
            }
        }
    }
}

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
    holder.SaveableStateProvider(nav.nativeKey()) {
        NativeTab(nav, home, settings, appearance, chat)
    }
}

@Composable
private fun NativeTab(
    nav: AppNavigation, home: HomeViewModel, settings: SettingsViewModel,
    appearance: AppearanceViewModel, chat: ChatViewModel,
) {
    val pages = nav.pages
    if (nav.settingsOverlay) BackHandler {
        if (nav.settingsPage == SettingsSubpage.MAIN) nav.settingsOverlay = false
        else nav.settingsPage = nav.settingsPage.parent()
    }
    when {
        nav.settingsOverlay -> SettingsPage(
            pages.plugins, settings, home, appearance, chat, nav.settingsPage,
            onClose = { nav.settingsOverlay = false },
            peersOpenRequests = nav.peersOpenRequests,
            onPeersOpenRequestsHandled = { nav.peersOpenRequests = false },
            onContinueHandoff = { nav.chat.keyHandoffDeferred = false },
            open = { entry ->
                nav.settingsOverlay = false
                nav.plugins.open(entry)
            },
            checkInstalled = { nav.checkInstalled() },
        ) { nav.settingsPage = it }
        pages.appsOpen || home.status is HomeStatus.NotConfigured -> HomeRoute(
            pages.core, home, pages.plugins, nav.plugins, nav.appsPage,
            { nav.appsPage = it },
            chat.pendingConfirmations, nav::openCardTarget,
            onConnection = { nav.settings(SettingsSubpage.SERVER) }, settingsUpgrade = settings.serverUpgrade,
            onSettings = { nav.settings() },
        )
        else -> ChatRoute(
            pages.core, nav.chat, pages.plugins, nav.plugins,
            onAppLink = { nav.openLink(it) },
        ) { nav.openApps(HomePage.LAUNCHER) }
    }
}

@Composable
private fun OpenDevWebEffect(url: String?, plugins: PluginsState, pages: PagesState) {
    val installed = plugins.viewModel.installed
    var handled by remember(url) { mutableStateOf(false) }
    LaunchedEffect(url, installed) {
        if (url != null && installed != null && !handled) {
            val plugin = installed.plugins.firstOrNull { it.webUrl == url }
            if (plugin != null) {
                handled = true
                plugin.tabChoices.firstOrNull()?.let { pages.send(Action.Pages(PageAction.OpenPage(it.tab))) }
            }
        }
    }
}

@Composable
private fun WithPendingCards(
    cards: List<PendingConfirmation>, fullscreen: Boolean,
    onOpen: (String?) -> Unit,
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
    LaunchedEffect(plugins, locales, shown) {
        plugins.refresh(Action.Settings(SettingsAction.SetLanguages(coreLanguageTags(shown, locales))))
    }
}

@Composable
fun StartupFailure(reason: String) {
    WontonTheme(defaultAppearance()) {
        Box(Modifier.fillMaxSize().padding(24.dp)) { Text(reason, color = MaterialTheme.colorScheme.error) }
    }
}
