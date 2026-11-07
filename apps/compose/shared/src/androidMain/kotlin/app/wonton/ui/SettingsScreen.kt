package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
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
    onSubpage: (SettingsSubpage) -> Unit,
) {
    val core = plugins.core
    val back = { onSubpage(subpage.parent()) }
    val version = appVersion()
    var rejection by remember { mutableStateOf<Problem?>(null) }
    BackHandler(enabled = subpage != SettingsSubpage.MAIN, onBack = back)
    when (subpage) {
        SettingsSubpage.TABS -> TabsSettingsScreen(LocalTabs.current.viewModel, back)
        SettingsSubpage.BACKUP -> ChatBackupScreen(
            chat.encryption,
            onAction = { action -> rejection = core.send(action) },
            onClose = back,
        )
        SettingsSubpage.PROFILE -> OwnerProfilePage(plugins, back)
        SettingsSubpage.APPEARANCE -> WontonScreen(bar = { WontonPushedBar("外观", back) }) {
            AppearanceScreen(core, appearance)
        }
        SettingsSubpage.NOTIFICATIONS -> NotificationSettingsScreen(core, settings, back)
        SettingsSubpage.ABOUT -> AboutPage(version, back)
        SettingsSubpage.PEERS -> PeersPage(core, back, peersOpenRequests, onPeersOpenRequestsHandled)
        SettingsSubpage.MAIN -> WontonScreen(bar = {
            val title = tabText("设置", "Settings")
            if (onClose != null) WontonPushedBar(title, onClose) else WontonRootBar(title)
        }) { SettingsScreen(settings, version, onSubpage) }
        else -> ServerSettingsPage(plugins, home, chat, subpage, onSubpage)
    }
    chat.problem?.let { ProblemDialog(it.problem) { core.send(Action.DismissChatProblem) } }
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
            OnboardingProblem(problem.onboardingTitle(), problem.message) { onboarding.send(Action.RefreshOnboarding) }
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
        SettingsEntry.Profile -> WontonRow("你的资料", showsForward = true) { onSubpage(SettingsSubpage.PROFILE) }
        is SettingsEntry.Server -> WontonRow(
            (entry.name ?: ServerName.Default(1u)).displayName(), entry.status.connectionText,
            showsForward = true, badge = entry.upgrade,
            onClick = { onSubpage(SettingsSubpage.SERVER) },
        )
        SettingsEntry.Appearance -> WontonRow("外观", showsForward = true) { onSubpage(SettingsSubpage.APPEARANCE) }
        SettingsEntry.Language -> LanguageSettingsRow()
        // 服务器配对（ADR-0050）；核心只对主人设备取到过列表才给这一项。
        is SettingsEntry.Peers -> WontonRow(
            stringResource(R.string.peers_title), entry.count.toString(),
            showsForward = true, onClick = { onSubpage(SettingsSubpage.PEERS) },
        )
        SettingsEntry.BottomBar -> WontonRow(tabText("底栏", "Tab bar"), showsForward = true) {
            onSubpage(SettingsSubpage.TABS)
        }
        is SettingsEntry.Notifications -> WontonRow(
            "通知", if (notificationsEnabled()) entry.push.notificationValue else "已关闭", showsForward = true,
            onClick = { onSubpage(SettingsSubpage.NOTIFICATIONS) },
        )
        SettingsEntry.DefaultAgent -> WontonRow(
            "默认 Agent", onboarding.view.agents.firstOrNull { it.selected }?.title,
            showsForward = true, onClick = onChooseAgent,
        )
        is SettingsEntry.ChatBackup -> WontonRow("聊天记录备份", entry.recovery.backupValue, showsForward = true) {
            onSubpage(SettingsSubpage.BACKUP)
        }
        SettingsEntry.RestartOnboarding -> WontonRow("重看引导") { onboarding.send(Action.RestartOnboarding) }
        SettingsEntry.About -> WontonRow("关于", version, showsForward = true) { onSubpage(SettingsSubpage.ABOUT) }
    }
}

@Composable
private fun AboutPage(version: String, onBack: () -> Unit) {
    WontonScreen(bar = { WontonPushedBar("关于", onBack) }) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset)) {
            WontonGroup { WontonRow("版本", version) }
        }
    }
}

internal val SettingsViewModel.serverUpgrade: Boolean
    get() = sections.any { section -> section.entries.any { it is SettingsEntry.Server && it.upgrade } }
