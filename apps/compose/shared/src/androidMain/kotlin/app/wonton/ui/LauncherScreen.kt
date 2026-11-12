package app.wonton.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PluginState
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Store
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WarningCircleFill
import app.wonton.ui.icons.Web
import app.wonton.ui.icons.WontonIcon

/** 配上以后的“应用”：独立的应用商店入口和已装插件网格。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LauncherScreen(
    home: HomeViewModel,
    plugins: PluginsState,
    navigation: PluginNavigation,
    pendingCards: List<PendingConfirmation>,
    onOpenCardRoom: (String) -> Unit,
    onShowStore: () -> Unit,
    onConnection: () -> Unit,
    settingsUpgrade: Boolean,
    onSettings: (() -> Unit)? = null,
) {
    LaunchedEffect(plugins) { plugins.refresh(Action.RefreshPlugins) }
    WontonScreen(
        bar = {
            WontonRootBar("应用", actions = if (onSettings != null) {
                { SettingsToolbarButton(settingsUpgrade, onSettings) }
            } else null)
        },
    ) {
        Column(Modifier.fillMaxSize()) {
            val installed = plugins.viewModel.installed
            StoreEntry(installed?.plugins?.count { it.upgrade != null }, onShowStore)
            PullToRefreshBox(
                isRefreshing = plugins.viewModel.refreshing,
                onRefresh = { plugins.send(Action.RefreshPlugins) },
                modifier = Modifier.fillMaxWidth().weight(1f),
            ) {
                if (installed == null) {
                    LauncherPlaceholder(plugins)
                } else {
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(WontonTheme.size.launcherCell),
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(WontonTheme.spacing.pageInset),
                        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s24),
                        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                    ) {
                        plugins.viewModel.problem?.let { problem ->
                            item(key = "problem", span = { GridItemSpan(maxLineSpan) }) {
                                ProblemRow(problem.problem) { plugins.send(Action.RefreshPlugins) }
                            }
                        }
                        installed.upgradeCheck?.let { check ->
                            item(key = "check", span = { GridItemSpan(maxLineSpan) }) {
                                ProblemRow(check.problem) { plugins.send(Action.RefreshPlugins) }
                            }
                        }
                        if (installed.plugins.isEmpty()) {
                            item(key = "empty", span = { GridItemSpan(maxLineSpan) }) { NoPlugins(onShowStore) }
                        }
                        items(installed.plugins, key = { it.id }) { LauncherIcon(it, plugins, navigation) }
                        item(key = "status", span = { GridItemSpan(maxLineSpan) }) {
                            HomeStatusPanel(home, plugins.viewModel, pendingCards, onConnection, onOpenCardRoom)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun NoPlugins(onShowStore: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(top = WontonTheme.spacing.s48),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            "还没有装应用",
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonButton("去应用商店", onShowStore, kind = WontonButtonKind.SECONDARY)
    }
}

/** 还没取到已安装的插件：取不到的原因和下一步，或者正在取。 */
@Composable
private fun LauncherPlaceholder(plugins: PluginsState) {
    val problem = plugins.viewModel.problem
    val rejection = plugins.refreshRejection
    val retry = { plugins.send(Action.RefreshPlugins) }
    when {
        problem != null -> ProblemView(problem.problem, retry)
        rejection != null -> ProblemView(rejection.problem, retry)
        else -> LoadingView()
    }
}

@Composable
private fun StoreEntry(upgradeCount: Int?, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    Row(
        Modifier.padding(horizontal = WontonTheme.spacing.pageInset)
            .padding(top = WontonTheme.spacing.pageInset, bottom = WontonTheme.spacing.s8).fillMaxWidth()
            .accentFill(shape).clickable(role = Role.Button, onClick = onClick)
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Icon(
            WontonIcon.Store, contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon), tint = colors.accentInkOnFill.toColor(),
        )
        Text(
            "应用商店", Modifier.weight(1f), style = WontonTheme.typography.body,
            color = colors.textOnAccentFill.toColor(),
        )
        if (upgradeCount != null && upgradeCount > 0) {
            Text(
                "$upgradeCount 个可更新", style = WontonTheme.typography.supporting,
                color = colors.textOnAccentFill.toColor(),
            )
        }
    }
}

/**
 * 主屏上的一个插件：圆角方形的图标和名字。能升级的、出了问题的在图标角上标出来，正在装、升级、卸载的转圈。
 * 点图标：Agent 插件进和它的 Bot 的私聊，有网页的打开小程序；都没有、或者正在装、升级、卸载时看详情。长按是菜单。
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun LauncherIcon(plugin: InstalledPlugin, plugins: PluginsState, navigation: PluginNavigation) {
    var showingMenu by remember { mutableStateOf(false) }
    var choosingPage by remember { mutableStateOf(false) }
    val tabs = LocalTabs.current
    val entry = plugin.entries.firstOrNull()?.takeUnless { plugin.isBusy }
    val size = WontonTheme.size.launcherIcon
    Column(
        Modifier.combinedClickable(
            onClick = {
                if (entry != null) navigation.open(entry) else navigation.detail = PluginRoute.Installed(plugin.id)
            },
            onLongClick = { showingMenu = true },
        ),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box {
            PluginIconImage(plugin.icon, size, plugins)
            if (plugin.isBusy) {
                Box(
                    Modifier.size(size).background(Color.Black.copy(alpha = 0.35f), pluginIconShape(size)),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.icon),
                        color = Color.White,
                        strokeWidth = 2.dp,
                    )
                }
            }
            Badge(plugin, Modifier.align(Alignment.TopEnd).offset(x = 6.dp, y = (-6).dp))
        }
        Text(
            plugin.title,
            style = WontonTheme.typography.caption,
            color = WontonTheme.colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
    if (showingMenu) LauncherMenu(plugin, navigation, onPin = {
        val choices = plugin.tabChoices.filter { choice -> tabs.viewModel.items.none { it.tab == choice.tab } }
        if (choices.size == 1) tabs.pin(choices.single().tab, choices.single().title)
        else if (choices.isNotEmpty()) choosingPage = true
    }) { showingMenu = false }
    if (choosingPage) TabPageChoiceSheet(plugin) { choosingPage = false }
}

@Composable
private fun Badge(plugin: InstalledPlugin, modifier: Modifier) {
    val colors = WontonTheme.colors
    when {
        plugin.status is PluginState.Degraded -> Icon(
            WontonIcon.WarningCircleFill,
            contentDescription = null,
            modifier = modifier.size(WontonTheme.size.iconSmall).background(colors.canvas.toColor(), CircleShape),
            tint = colors.danger.toColor(),
        )
        plugin.upgrade != null -> Box(
            modifier.size(12.dp)
                .background(colors.primary.toColor(), CircleShape)
                .border(2.dp, colors.canvas.toColor(), CircleShape),
        )
    }
}

@Composable
private fun LauncherMenu(
    plugin: InstalledPlugin,
    navigation: PluginNavigation,
    onPin: () -> Unit,
    onDismiss: () -> Unit,
) {
    val pinLabel = tabText("固定到底栏", "Pin to tab bar")
    val tabs = LocalTabs.current
    val canPin = plugin.tabChoices.any { choice -> tabs.viewModel.items.none { it.tab == choice.tab } }
    val actions = buildList {
        if (canPin) add(WontonSheetAction(pinLabel, WontonIcon.Web, onClick = onPin))
        add(WontonSheetAction("详情", WontonIcon.Info) { navigation.detail = PluginRoute.Installed(plugin.id) })
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        plugin.entries.takeIf { it.size > 1 }?.last()?.let { web ->
            add(WontonSheetAction("打开网页", WontonIcon.Web) { navigation.open(web) })
        }
        plugin.upgrade?.let { upgrade ->
            add(
                WontonSheetAction("升级到 ${upgrade.version}", WontonIcon.Download) {
                    navigation.confirmation = PluginConfirmation.Upgrade(plugin.id)
                },
            )
        }
        if (plugin.removable) {
            add(
                WontonSheetAction("卸载", WontonIcon.Trash, isDanger = true) {
                    navigation.uninstalling = UninstallTarget(plugin.id, plugin.title)
                },
            )
        }
    }
    WontonActionSheet(actions, onDismiss, plugin.title)
}
