package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfoV2
import androidx.compose.material3.adaptive.layout.ListDetailPaneScaffoldRole
import androidx.compose.material3.adaptive.layout.ThreePaneScaffoldDestinationItem
import androidx.compose.material3.adaptive.layout.calculatePaneScaffoldDirective
import androidx.compose.material3.adaptive.navigation.NavigableListDetailPaneScaffold
import androidx.compose.material3.adaptive.navigation.rememberListDetailPaneScaffoldNavigator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.window.core.layout.WindowSizeClass
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.BackupReminder
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.TabRef
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PanelScope
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Apps
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.Store
import app.wonton.ui.icons.WontonIcon

internal val LocalExpandedChat = compositionLocalOf { false }

@OptIn(ExperimentalMaterial3AdaptiveApi::class)
@Composable
internal fun AdaptiveChatLayout(
    list: @Composable () -> Unit,
    enabled: Boolean = true,
    content: @Composable () -> Unit,
) {
    val info = currentWindowAdaptiveInfoV2()
    val expanded = enabled &&
        info.windowSizeClass.isWidthAtLeastBreakpoint(WindowSizeClass.WIDTH_DP_EXPANDED_LOWER_BOUND)
    val directive = calculatePaneScaffoldDirective(info).copy(
        maxHorizontalPartitions = if (expanded) 2 else 1,
        maxVerticalPartitions = 1,
        horizontalPartitionSpacerSize = WontonTheme.size.hairline,
    )
    // 导航仍归 ChatNavigation / PagesState；scaffold 只决定布局，不另建房间或插件返回栈。
    // 窄屏也留在同一 detail 槽，尺寸变化不会销毁网页实例或房间输入状态。
    val navigator = rememberListDetailPaneScaffoldNavigator<String>(
        scaffoldDirective = directive,
        initialDestinationHistory = listOf(ThreePaneScaffoldDestinationItem(ListDetailPaneScaffoldRole.Detail)),
    )
    CompositionLocalProvider(LocalExpandedChat provides expanded) {
        NavigableListDetailPaneScaffold(
            navigator = navigator,
            listPane = { if (expanded) Box(Modifier.fillMaxSize().testTag("wide-screen-list")) { list() } },
            detailPane = {
                Box(Modifier.fillMaxSize().then(if (expanded) Modifier.testTag("wide-screen-detail") else Modifier)) {
                    content()
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
    }
}

@Composable
internal fun ExpandedChatPane(nav: AppNavigation, chat: ChatViewModel, home: HomeViewModel, appearance: AppearanceViewModel) {
    val panel = rememberChatPanelState(PanelEdge.TOP, PanelScope.Main)
    val draft = LocalTabDraft.current
    SideEffect { panel.feed(nav.pages.viewModel.panel, nav.pages.viewModel.panelSelected) }
    CompositionLocalProvider(LocalChatPanel provides panel) {
        ChatPanelHost(panel, nav.pages, home, appearance,
            onDraft = { text -> draft.text = text; nav.pages.send(Action.OpenPage(TabRef.Chat)) },
            onDetails = nav::detail) {
            ExpandedChatList(nav, chat)
        }
    }
    if (panel.editing) ChatPanelEditor(panel, nav.pages) { panel.editing = false }
}

@Composable
internal fun ExpandedChatList(nav: AppNavigation, chat: ChatViewModel) {
    var appsExpanded by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        ExpandedChatActions({ nav.sidebar(SidebarDestination.NewChat) }, { appsExpanded = !appsExpanded },
            { nav.sidebar(SidebarDestination.Store) }, { nav.settings() })
        if (appsExpanded) {
            Column(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.s12)) {
                SidebarApps(nav.pages.plugins, nav.pages, nav.pages.current, true, nav::sidebar)
            }
        }
        Box(Modifier.weight(1f)) {
            ChatListScreen(
                chat, nav.pages.plugins, nav.plugins,
                onAction = { nav.pages.plugins.reject(nav.pages.core.send(it)) },
                onOpenRoom = {
                    nav.chat.startingChat = false
                    nav.openRoom(it)
                },
                onOpenBackup = {
                    if (chat.encryption.backupReminder == BackupReminder.CONTINUE_ON_OTHER_DEVICE) {
                        nav.chat.keyHandoffDeferred = false
                    } else {
                        nav.settingsOverlay = false
                        nav.closeApps()
                        nav.pages.send(Action.OpenPage(TabRef.Chat))
                        nav.chat.openRoom = null
                        nav.chat.showingBackup = true
                    }
                },
                onReviewInstall = {
                    nav.settingsOverlay = false
                    nav.openApps(HomePage.LAUNCHER)
                },
                onStartChat = { nav.sidebar(SidebarDestination.NewChat) },
                onOpenApps = { nav.openApps(HomePage.LAUNCHER) },
                showNewChat = false,
                selectedRoom = nav.chat.openRoom.takeIf {
                    nav.pages.current == TabRef.Chat && !nav.settingsOverlay && nav.appsPage == null
                },
            )
        }
    }
}

@Composable
internal fun ExpandedChatActions(onNewChat: () -> Unit, onApps: () -> Unit, onStore: () -> Unit, onSettings: () -> Unit) {
    Row {
        IconButton(onClick = onNewChat) { Icon(WontonIcon.Compose, stringResource(R.string.sidebar_new_chat)) }
        IconButton(onClick = onApps) { Icon(WontonIcon.Apps, stringResource(R.string.sidebar_my_apps)) }
        IconButton(onClick = onStore) { Icon(WontonIcon.Store, stringResource(R.string.sidebar_store)) }
        IconButton(onClick = onSettings) { Icon(WontonIcon.Settings, stringResource(R.string.sidebar_settings)) }
    }
}

@Composable
internal fun ChatDetailEmpty() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text(stringResource(R.string.tab_chat), style = WontonTheme.typography.screenTitle,
            color = WontonTheme.colors.textSecondary.toColor())
    }
}
