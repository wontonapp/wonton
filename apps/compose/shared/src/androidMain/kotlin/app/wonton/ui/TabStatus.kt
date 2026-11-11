package app.wonton.ui

import android.icu.text.ListFormatter
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.webkit.WebViewCompat
import app.wonton.protocol.Action
import app.wonton.protocol.PlacedTab
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
    crossOriginBlocked: Boolean, onDetails: (String) -> Unit, place: PlacedTab? = null,
) {
    val state = page.state
    val isolationWarning = crossOriginBlocked && supported && problem == null &&
        (state is TabState.Ready || state is TabState.Upgrading)
    val failure = when {
        !supported -> Problem(stringResource(R.string.tab_webview_too_old))
        problem != null -> problem
        state is TabState.Stopped -> Problem(stringResource(R.string.tab_stopped, page.title))
        state is TabState.Failed -> Problem(stringResource(R.string.tab_problem, page.title), detail = state.reason)
        state is TabState.PageRemoved -> Problem(stringResource(R.string.tab_page_removed))
        state is TabState.Unknown -> Problem(stringResource(R.string.tab_status_unknown))
        state is TabState.LoadFailed -> Problem(
            loadFailureTitle(state.failure), detail = state.status?.let { "HTTP $it" },
        )
        isolationWarning -> crossOriginIsolationProblem()
        else -> null
    }
    if (failure == null) {
        Column(Modifier.fillMaxSize()) {
            if (state is TabState.Starting || state is TabState.Uninstalling) Text(
                stringResource(if (state is TabState.Starting) R.string.tab_starting else R.string.tab_uninstalling),
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
            Text(failure.titleText, style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor())
            failure.detail?.let { ProblemDetail(it, stringResource(R.string.common_details)) }
            TabStatusActions(state, tab, tabs, isolationWarning, onDetails, place)
        }
    }
}

@Composable
private fun TabStatusActions(
    state: TabState, tab: TabRef, tabs: TabsState, crossOriginBlocked: Boolean,
    onDetails: (String) -> Unit, place: PlacedTab?,
) {
    if (crossOriginBlocked && tab is TabRef.Plugin) {
        WontonButton(stringResource(R.string.tab_open_anyway), { allowCrossOriginIsolationAnyway(tab.pluginId) })
    } else if (state is TabState.PageRemoved && place is PlacedTab.Panel) {
        WontonButton(stringResource(R.string.panel_remove), {
            tabs.send(Action.RemovePanelPage(place.scope, tab))
        })
    } else if (state is TabState.PageRemoved) {
        val pinned = tabs.viewModel.items.any { it.tab == tab }
        val label = if (pinned) stringResource(R.string.tab_unpin_from_bar) else stringResource(R.string.common_close)
        WontonButton(label, {
            tabs.send(if (pinned) Action.UnpinTab(tab) else Action.CloseTabOverlay)
        })
    } else if (state is TabState.Stopped && tab is TabRef.Plugin) {
        WontonButton(stringResource(R.string.tab_start), { tabs.send(Action.StartPlugin(tab.pluginId)) })
    } else {
        WontonButton(stringResource(R.string.common_retry), {
            tabs.send(Action.ReloadTab(place ?: PlacedTab.Bar(tab)))
        })
        if (state is TabState.Failed && tab is TabRef.Plugin) {
            WontonButton(stringResource(R.string.tab_view_details), { onDetails(tab.pluginId) })
        }
    }
}

@Composable
private fun crossOriginIsolationProblem(): Problem {
    val context = LocalContext.current
    val currentVersion = remember(context) { WebViewCompat.getCurrentWebViewPackage(context)?.versionName }
    val version = currentVersion ?: stringResource(R.string.tab_webview_version_unknown)
    return Problem(
        stringResource(R.string.tab_cross_origin_title),
        detail = stringResource(R.string.tab_cross_origin_detail, version),
    )
}

@Composable
private fun loadFailureTitle(failure: TabLoadFailure): String = when (failure) {
    TabLoadFailure.TRANSPORT -> stringResource(R.string.problem_transport_unavailable_title)
    TabLoadFailure.PLUGIN_NOT_READY -> stringResource(R.string.tab_load_not_ready)
    TabLoadFailure.UPSTREAM -> stringResource(R.string.tab_load_upstream)
}

@Composable
fun TabNotices(tabs: TabsState, onSettings: () -> Unit) {
    val view = tabs.viewModel
    var dismissed by remember { mutableStateOf<String?>(null) }
    if (view.uninstalled.isNotEmpty()) {
        val names = ListFormatter.getInstance(LocalConfiguration.current.locales[0]).format(view.uninstalled)
        val dismiss = { tabs.send(Action.DismissUninstalledTabs) }
        WontonDialog(
            stringResource(R.string.tab_uninstalled_notice, names),
            onDismiss = dismiss,
            actions = listOf(WontonDialogAction(stringResource(R.string.design_ok), onClick = dismiss)),
        )
    } else view.problem?.takeUnless { it.message == dismissed }?.let { problem ->
        WontonDialog(
            stringResource(R.string.tab_settings_unreadable),
            onDismiss = { dismissed = problem.message },
            actions = listOf(WontonDialogAction(stringResource(R.string.tab_open_settings)) {
                dismissed = problem.message
                onSettings()
            }),
            detail = { ProblemDetail(problem.message, stringResource(R.string.common_details)) }
        )
    }
}
