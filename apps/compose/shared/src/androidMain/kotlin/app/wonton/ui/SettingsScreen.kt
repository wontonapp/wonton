package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.ChatAction
import app.wonton.protocol.FriendsAction
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.ServerName
import app.wonton.protocol.SettingsEntry
import app.wonton.protocol.SettingsViewModel
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme

@Composable
fun SettingsPage(
    plugins: PluginsState,
    settings: SettingsViewModel,
    home: HomeViewModel,
    appearance: AppearanceViewModel,
    chat: ChatViewModel,
    subpage: SettingsSubpage,
    onClose: (() -> Unit)? = null,
    peersOpenRequests: Boolean = false,
    onPeersOpenRequestsHandled: () -> Unit = {},
    onContinueHandoff: () -> Unit,
    onSubpage: (SettingsSubpage) -> Unit,
) {
    val core = plugins.core
    val appLock by remember(core) { core.appLockUpdates() }.collectAsState(remember(core) { core.appLock() })
    // 好友项只在列表取到过以后出现；进设置先取一次。
    LaunchedEffect(core) { core.send(Action.Friends(FriendsAction.RefreshFriends)) }
    val back = { onSubpage(subpage.parent()) }
    val version = appVersion()
    var rejection by remember { mutableStateOf<Problem?>(null) }
    BackHandler(enabled = subpage != SettingsSubpage.MAIN, onBack = back)
    when (subpage) {
        SettingsSubpage.MY_APPS -> MyAppsSettingsScreen(back)
        SettingsSubpage.BACKUP -> ChatBackupScreen(
            chat.encryption,
            onAction = { action -> rejection = core.send(action) },
            onContinueHandoff = onContinueHandoff,
            onClose = back,
        )
        SettingsSubpage.PROFILE -> OwnerProfilePage(plugins, back)
        SettingsSubpage.APPEARANCE -> WontonScreen(bar = {
            WontonPushedBar(stringResource(R.string.settings_appearance), back)
        }) {
            AppearanceScreen(core, appearance)
        }
        SettingsSubpage.NOTIFICATIONS -> NotificationSettingsScreen(core, settings, back)
        SettingsSubpage.APP_LOCK -> AppLockSettingsScreen(core, appLock, back)
        SettingsSubpage.ABOUT -> AboutPage(version, back)
        SettingsSubpage.PEERS -> PeersPage(core, back, peersOpenRequests, onPeersOpenRequestsHandled)
        SettingsSubpage.FRIENDS -> FriendsPage(core, back)
        SettingsSubpage.MAIN -> WontonScreen(bar = {
            val title = stringResource(R.string.tab_settings)
            if (onClose != null) WontonPushedBar(title, onClose) else WontonRootBar(title)
        }) { SettingsScreen(settings, version, onSubpage) }
        else -> ServerSettingsPage(plugins, home, chat, subpage, onSubpage)
    }
    chat.problem?.let { ProblemDialog(it.problem) { core.send(Action.Chat(ChatAction.DismissChatProblem)) } }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

@Composable
private fun SettingsScreen(settings: SettingsViewModel, version: String, onSubpage: (SettingsSubpage) -> Unit) {
    val onboarding = LocalOnboarding.current
    var choosingAgent by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        settings.sections.forEach { section ->
            WontonGroup {
                section.entries.forEachIndexed { index, entry ->
                    if (index > 0) WontonSeparator()
                    SettingsEntryRow(entry, version, onSubpage) { choosingAgent = true }
                }
            }
        }
        onboarding.view.problem?.let { problem ->
            OnboardingProblem(problem.onboardingTitle(), problem.message) {
                onboarding.send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
            }
        }
    }
    if (choosingAgent) DefaultAgentSelection { choosingAgent = false }
}

@Composable
private fun SettingsEntryRow(
    entry: SettingsEntry,
    version: String,
    onSubpage: (SettingsSubpage) -> Unit,
    onChooseAgent: () -> Unit,
) {
    val onboarding = LocalOnboarding.current
    when (entry) {
        SettingsEntry.Profile -> WontonRow(stringResource(R.string.settings_profile), showsForward = true) {
            onSubpage(SettingsSubpage.PROFILE)
        }
        is SettingsEntry.Server -> WontonRow(
            (entry.name ?: ServerName.Default(1u)).displayName(), entry.status.connectionText,
            showsForward = true, badge = entry.upgrade,
            onClick = { onSubpage(SettingsSubpage.SERVER) },
        )
        SettingsEntry.Appearance -> WontonRow(stringResource(R.string.settings_appearance), showsForward = true) {
            onSubpage(SettingsSubpage.APPEARANCE)
        }
        SettingsEntry.Language -> LanguageSettingsRow()
        // 服务器配对（ADR-0050）；核心只对主人设备取到过列表才给这一项。
        is SettingsEntry.Peers -> WontonRow(
            stringResource(R.string.peers_title), entry.count.toString(),
            showsForward = true, onClick = { onSubpage(SettingsSubpage.PEERS) },
        )
        // 好友（ADR-0051）；核心只在列表取到过时给这一项，请求数做角标。
        is SettingsEntry.Friends -> WontonRow(
            stringResource(R.string.friends_title),
            entry.count.toString(),
            showsForward = true,
            badgeCount = entry.requests.toInt(),
            onClick = { onSubpage(SettingsSubpage.FRIENDS) },
        )
        SettingsEntry.MyApps -> WontonRow(stringResource(R.string.settings_my_apps), showsForward = true) {
            onSubpage(SettingsSubpage.MY_APPS)
        }
        is SettingsEntry.AppLock -> WontonRow(
            stringResource(R.string.applock_title),
            if (entry.enabled) stringResource(appLockTimeoutLabel(entry.timeout)) else null,
            showsForward = true,
            onClick = { onSubpage(SettingsSubpage.APP_LOCK) },
        )
        is SettingsEntry.Notifications -> WontonRow(
            stringResource(R.string.notification_title),
            // 摘要和通知页的“系统通知”一行同源：收不收得到提醒只看系统有没有允许，推送注册状态在通知页里说。
            stringResource(
                if (notificationsEnabled()) R.string.notification_value_on
                else R.string.notification_value_system_blocked,
            ),
            showsForward = true,
            onClick = { onSubpage(SettingsSubpage.NOTIFICATIONS) },
        )
        SettingsEntry.DefaultAgent -> WontonRow(
            stringResource(R.string.settings_default_agent), onboarding.view.agents.firstOrNull { it.selected }?.title,
            showsForward = true, onClick = onChooseAgent,
        )
        is SettingsEntry.ChatBackup -> WontonRow(
            stringResource(R.string.chat_backup_title), entry.recovery.backupValue, showsForward = true,
        ) {
            onSubpage(SettingsSubpage.BACKUP)
        }
        SettingsEntry.RestartOnboarding -> WontonRow(stringResource(R.string.settings_show_setup_again)) {
            onboarding.send(Action.Onboarding(OnboardingAction.RestartOnboarding))
        }
        SettingsEntry.About -> WontonRow(stringResource(R.string.settings_about), version, showsForward = true) {
            onSubpage(SettingsSubpage.ABOUT)
        }
        // 外部 Matrix 账号和添加 Wonton 服务器的入口（ADR-0051）：账号流程还没建，核心从不下发这几项，
        // 和 iOS 一样先不渲染；显式列出，核心开始下发时不至于静默漏掉。
        is SettingsEntry.MatrixAccount,
        is SettingsEntry.MatrixDevices,
        SettingsEntry.AddWontonServer -> Unit
        // 未知入口不显示：新核心加的按钮不能变成空行。
        else -> Unit
    }
}

@Composable
private fun AboutPage(version: String, onBack: () -> Unit) {
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.settings_about), onBack) }) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset)) {
            WontonGroup { WontonRow(stringResource(R.string.settings_version), version) }
        }
    }
}

internal val SettingsViewModel.serverUpgrade: Boolean
    get() = sections.any { section -> section.entries.any { it is SettingsEntry.Server && it.upgrade } }
