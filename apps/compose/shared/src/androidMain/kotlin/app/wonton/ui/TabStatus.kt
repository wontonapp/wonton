package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.webkit.WebViewCompat
import app.wonton.protocol.Action
import app.wonton.protocol.TabLoadFailure
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun PluginTabStatus(
    page: TabPage, tab: TabRef, tabs: TabsState, problem: Problem?, supported: Boolean,
    crossOriginBlocked: Boolean, onDetails: (String) -> Unit,
) {
    val state = page.state
    val isolationWarning = crossOriginBlocked && supported && problem == null &&
        (state is TabState.Ready || state is TabState.Upgrading)
    val failure = when {
        !supported -> Problem(tabText("请更新 Android System WebView 后重试", "Update Android System WebView and try again"))
        problem != null -> problem
        state is TabState.Stopped -> Problem(tabText("${page.title}已停止", "${page.title} has stopped"))
        state is TabState.Failed -> Problem(
            tabText("${page.title}出了问题", "${page.title} has a problem"), detail = state.reason,
        )
        state is TabState.PageRemoved -> Problem(tabText("新版本不再提供这个页面", "This page is no longer available"))
        state is TabState.Unknown -> Problem(tabText("无法确定应用状态", "App status is unknown"))
        state is TabState.LoadFailed -> Problem(
            loadFailureTitle(state.failure), detail = state.status?.let { "HTTP $it" },
        )
        isolationWarning -> crossOriginIsolationProblem()
        else -> null
    }
    if (failure == null) {
        Column(Modifier.fillMaxSize()) {
            if (state is TabState.Starting || state is TabState.Uninstalling) Text(
                if (state is TabState.Starting) tabText("正在启动…", "Starting…") else tabText("正在卸载…", "Uninstalling…"),
                Modifier.padding(WontonTheme.spacing.pageInset), color = WontonTheme.colors.textSecondary.toColor(),
            )
            ListSkeleton(WontonTheme.size.storeIcon)
        }
    } else {
        Column(
            Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(failure.title, style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor())
            failure.detail?.let { ProblemDetail(it, tabText("详情", "Details")) }
            TabStatusActions(state, tab, tabs, isolationWarning, onDetails)
        }
    }
}

@Composable
private fun TabStatusActions(
    state: TabState, tab: TabRef, tabs: TabsState, crossOriginBlocked: Boolean,
    onDetails: (String) -> Unit,
) {
    if (crossOriginBlocked && tab is TabRef.Plugin) {
        WontonButton(tabText("仍然打开", "Open anyway"), { allowCrossOriginIsolationAnyway(tab.pluginId) })
    } else if (state is TabState.PageRemoved) {
        val pinned = tabs.viewModel.items.any { it.tab == tab }
        WontonButton(if (pinned) tabText("从底栏移除", "Remove from tab bar") else tabText("关闭", "Close"), {
            tabs.send(if (pinned) Action.UnpinTab(tab) else Action.CloseTabOverlay)
        })
    } else if (state is TabState.Stopped && tab is TabRef.Plugin) {
        WontonButton(tabText("启动", "Start"), { tabs.send(Action.StartPlugin(tab.pluginId)) })
    } else {
        WontonButton(tabText("重试", "Retry"), { tabs.send(Action.ReloadTab(tab)) })
        if (state is TabState.Failed && tab is TabRef.Plugin) {
            WontonButton(tabText("查看详情", "View details"), { onDetails(tab.pluginId) })
        }
    }
}

@Composable
private fun crossOriginIsolationProblem(): Problem {
    val context = LocalContext.current
    val currentVersion = remember(context) { WebViewCompat.getCurrentWebViewPackage(context)?.versionName }
    val version = currentVersion ?: tabText("拿不到版本号", "version unavailable")
    return Problem(
        tabText("这个应用在 Android App 里暂时打不开", "This app can't open in the Android app yet"),
        detail = tabText(
            "当前 Android System WebView：$version；需要 156 或更新。",
            "Current Android System WebView: $version; requires 156 or newer.",
        ),
    )
}

@Composable
private fun loadFailureTitle(failure: TabLoadFailure): String = when (failure) {
    TabLoadFailure.TRANSPORT -> tabText("连不上家里的服务器", "Cannot connect to your home server")
    TabLoadFailure.PLUGIN_NOT_READY -> tabText("应用还没有准备好", "The app is not ready")
    TabLoadFailure.UPSTREAM -> tabText("应用的网页打不开", "The app page could not load")
}

@Composable
fun TabNotices(tabs: TabsState, onSettings: () -> Unit) {
    val view = tabs.viewModel
    var dismissed by remember { mutableStateOf<String?>(null) }
    if (view.uninstalled.isNotEmpty()) {
        val names = view.uninstalled.joinToString("、")
        val dismiss = { tabs.send(Action.DismissUninstalledTabs) }
        WontonDialog(
            tabText("$names 已卸载，已从底栏移除", "$names was uninstalled and removed from the tab bar"),
            onDismiss = dismiss,
            actions = listOf(WontonDialogAction(tabText("好", "OK"), onClick = dismiss)),
        )
    } else view.problem?.takeUnless { it.message == dismissed }?.let { problem ->
        WontonDialog(
            tabText("底栏设置读不了", "Could not read tab bar settings"),
            onDismiss = { dismissed = problem.message },
            actions = listOf(WontonDialogAction(tabText("查看底栏设置", "Tab bar settings")) {
                dismissed = problem.message
                onSettings()
            }),
            detail = { ProblemDetail(problem.message, tabText("详情", "Details")) }
        )
    }
}
