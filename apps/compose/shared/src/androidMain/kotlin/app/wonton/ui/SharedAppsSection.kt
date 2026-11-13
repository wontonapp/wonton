package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.SharedAppRow
import app.wonton.protocol.SharedAppsSource
import app.wonton.protocol.SharedAppsState
import app.wonton.protocol.SharedAppsViewModel
import app.wonton.protocol.TabState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun SharedAppsSection(view: SharedAppsViewModel?, plugins: PluginsState) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        Text(stringResource(R.string.share_received), style = WontonTheme.typography.sectionTitle,
            color = WontonTheme.colors.textPrimary.toColor())
        WontonButton(stringResource(R.string.share_refresh), { plugins.send(Action.RefreshSharedApps(null)) },
            kind = WontonButtonKind.SECONDARY)
        if (view == null) LoadingView()
        view?.sources?.forEach { source -> SharedSourceSection(source, plugins) }
    }
}

@Composable
private fun SharedSourceSection(source: SharedAppsSource, plugins: PluginsState) {
    val refresh = { plugins.send(Action.RefreshSharedApps(source.ownerUserId)) }
    DetailSection(source.ownerDisplayName ?: source.ownerUserId) {
        if (source.ownerDisplayName != null) Text(source.ownerUserId,
            style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
        when (val state = source.state) {
            SharedAppsState.Loading -> LoadingView()
            is SharedAppsState.Failed -> ProblemNotice(state.problem.asProblem())
            SharedAppsState.Other -> ProblemNotice(Problem(titleRes = R.string.share_problem_other))
            is SharedAppsState.Ready -> Unit
        }
        WontonButton(stringResource(R.string.share_refresh), refresh, kind = WontonButtonKind.SECONDARY)
        source.apps.forEach { row -> SharedAppEntry(source, row, plugins) }
    }
}

@Composable
private fun SharedAppEntry(source: SharedAppsSource, row: SharedAppRow, plugins: PluginsState) {
    val open = { plugins.send(Action.OpenSharedApp(source.ownerUserId, row.plugin.pluginId)) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(row.plugin.title, style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor())
        row.problem?.let { ProblemNotice(it.asProblem()) }
        sharedPluginProblem(row.plugin.state, row.plugin.title)?.let { ProblemNotice(it) }
        WontonButton(stringResource(if (row.problem == null) R.string.store_open else R.string.common_retry), open,
            kind = WontonButtonKind.SECONDARY, enabled = source.state is SharedAppsState.Ready)
    }
}

@Composable
internal fun sharedPluginProblem(state: TabState, title: String): Problem? = when (state) {
    is TabState.Stopped -> Problem(titleRes = R.string.share_problem_plugin)
    is TabState.Failed -> Problem(stringResource(R.string.tab_problem, title), detail = state.reason)
    is TabState.PageRemoved -> Problem(titleRes = R.string.tab_page_removed)
    is TabState.Unknown -> Problem(titleRes = R.string.tab_status_unknown)
    is TabState.Starting -> Problem(titleRes = R.string.tab_starting)
    is TabState.Uninstalling -> Problem(titleRes = R.string.tab_uninstalling)
    is TabState.Upgrading -> Problem(titleRes = R.string.tab_upgrading)
    is TabState.LoadFailed -> Problem(titleRes = when (state.failure) {
        app.wonton.protocol.TabLoadFailure.TRANSPORT -> R.string.problem_transport_unavailable_title
        app.wonton.protocol.TabLoadFailure.PLUGIN_NOT_READY -> R.string.tab_load_not_ready
        app.wonton.protocol.TabLoadFailure.UPSTREAM -> R.string.tab_load_upstream
    }, detail = state.details ?: state.status?.let { "HTTP $it" })
    is TabState.Ready, is TabState.Loading -> null
}
