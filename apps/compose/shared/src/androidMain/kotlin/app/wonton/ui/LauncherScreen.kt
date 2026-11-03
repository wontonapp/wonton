package app.wonton.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PluginState

private val ICON_SIZE = 62.dp

/** 配上以后的“家”：按图标和名字列出已装的插件。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LauncherScreen(
    status: HomeStatus,
    plugins: PluginsState,
    navigation: PluginNavigation,
    onShowStore: () -> Unit,
    onAppearance: () -> Unit,
    onConnection: () -> Unit,
) {
    LaunchedEffect(plugins) { plugins.refresh(Action.RefreshPlugins) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("家") },
                actions = {
                    TextButton(onClick = onAppearance) { Text("外观") }
                    TextButton(onClick = onConnection) {
                        Text("连接", color = if (status is HomeStatus.Disconnected) Red else Color.Unspecified)
                    }
                },
            )
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = plugins.viewModel.refreshing,
            onRefresh = { plugins.send(Action.RefreshPlugins) },
            modifier = Modifier.padding(padding).fillMaxSize(),
        ) {
            val installed = plugins.viewModel.installed
            if (installed == null) {
                LauncherPlaceholder(plugins)
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(76.dp),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(22.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    plugins.viewModel.problem?.let { problem ->
                        item(key = "problem", span = { GridItemSpan(maxLineSpan) }) {
                            ProblemRow(problem.message, problem.reason.nextStep)
                        }
                    }
                    installed.upgradeCheck?.let { check ->
                        item(key = "check", span = { GridItemSpan(maxLineSpan) }) { ProblemRow(check.message, null) }
                    }
                    if (installed.plugins.isEmpty()) {
                        item(key = "empty", span = { GridItemSpan(maxLineSpan) }) { NoPlugins(onShowStore) }
                    }
                    items(installed.plugins, key = { it.id }) { LauncherIcon(it, plugins, navigation) }
                }
            }
        }
    }
}

@Composable
private fun NoPlugins(onShowStore: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(top = 96.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("还没有装插件", style = MaterialTheme.typography.titleMedium)
        OutlinedButton(onClick = onShowStore) { Text("去商店") }
    }
}

/** 还没取到已安装的插件：取不到的原因和下一步，或者正在取。 */
@Composable
private fun LauncherPlaceholder(plugins: PluginsState) {
    val problem = plugins.viewModel.problem
    val rejection = plugins.refreshRejection
    val retry = { plugins.send(Action.RefreshPlugins) }
    when {
        problem != null -> ProblemView(problem.message, problem.reason.nextStep, retry)
        rejection != null -> ProblemView(rejection, null, retry)
        else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
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
    val entry = plugin.entries.firstOrNull()?.takeUnless { plugin.isBusy }
    Column(
        Modifier.combinedClickable(
            onClick = {
                if (entry != null) navigation.open(entry) else navigation.detail = PluginRoute.Installed(plugin.id)
            },
            onLongClick = { showingMenu = true },
        ),
        verticalArrangement = Arrangement.spacedBy(6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box {
            PluginIconImage(plugin.icon, plugin.title, ICON_SIZE, plugins)
            if (plugin.isBusy) {
                Box(
                    Modifier.size(ICON_SIZE).background(Color.Black.copy(alpha = 0.35f), pluginIconShape(ICON_SIZE)),
                    contentAlignment = Alignment.Center,
                ) { CircularProgressIndicator(Modifier.size(24.dp), color = Color.White, strokeWidth = 2.dp) }
            }
            Badge(plugin, Modifier.align(Alignment.TopEnd).offset(x = 6.dp, y = (-6).dp))
            LauncherMenu(plugin, navigation, expanded = showingMenu, onDismiss = { showingMenu = false })
        }
        Text(plugin.title, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun Badge(plugin: InstalledPlugin, modifier: Modifier) {
    val color = when {
        plugin.status is PluginState.Degraded -> Red
        plugin.upgrade != null -> Blue
        else -> return
    }
    Box(
        modifier.size(14.dp)
            .border(2.dp, MaterialTheme.colorScheme.surface, CircleShape)
            .padding(2.dp)
            .background(color, CircleShape),
    )
}

@Composable
private fun LauncherMenu(
    plugin: InstalledPlugin,
    navigation: PluginNavigation,
    expanded: Boolean,
    onDismiss: () -> Unit,
) {
    val choose = { action: () -> Unit ->
        onDismiss()
        action()
    }
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        DropdownMenuItem(
            text = { Text("详情") },
            onClick = { choose { navigation.detail = PluginRoute.Installed(plugin.id) } },
        )
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        plugin.entries.takeIf { it.size > 1 }?.last()?.let { web ->
            DropdownMenuItem(text = { Text("打开网页") }, onClick = { choose { navigation.open(web) } })
        }
        plugin.upgrade?.let { upgrade ->
            DropdownMenuItem(
                text = { Text("升级到 ${upgrade.version}") },
                onClick = { choose { navigation.confirmation = PluginConfirmation.Upgrade(plugin.id) } },
            )
        }
        if (plugin.removable) {
            DropdownMenuItem(
                text = { Text("卸载", color = Red) },
                onClick = { choose { navigation.uninstalling = UninstallTarget(plugin.id, plugin.title) } },
            )
        }
    }
}
