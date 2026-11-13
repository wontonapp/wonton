package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.SharedAppsSource
import app.wonton.protocol.SharedAppsState
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun SharedPageStatus(
    page: TabPage, tab: TabRef.SharedApp, source: SharedAppsSource?, tabs: PagesState,
    problem: Problem?, supported: Boolean, cover: Boolean, inset: PaddingValues,
) {
    val sourceFailure = (source?.state as? SharedAppsState.Failed)?.problem
    val row = source?.apps?.firstOrNull { it.plugin.pluginId == tab.pluginId }
    val rowFailure = row?.problem
    val failure = when {
        !supported -> Problem(titleRes = R.string.tab_webview_too_old)
        sourceFailure != null -> sourceFailure.asProblem()
        rowFailure != null -> rowFailure.asProblem()
        source == null || source.state is SharedAppsState.Other -> Problem(titleRes = R.string.share_problem_other)
        source.state is SharedAppsState.Ready && row == null -> Problem(titleRes = R.string.share_problem_required)
        problem != null -> problem
        else -> sharedPluginProblem(page.state, page.title)
    }
    if (!cover && failure == null && source?.state is SharedAppsState.Ready) return
    Column(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor()).padding(inset)
        .padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        if (failure != null) {
            ProblemNotice(failure)
            if (supported) WontonButton(stringResource(R.string.common_retry), {
                if (source?.state !is SharedAppsState.Ready || row == null || row.problem != null) {
                    tabs.send(Action.RefreshSharedApps(tab.ownerUserId))
                }
                tabs.send(Action.ReloadTab(PlacedTab.Page(tab)))
            }, kind = WontonButtonKind.SECONDARY)
        } else LoadingView()
        WontonButton(stringResource(R.string.share_refresh), {
            tabs.send(Action.RefreshSharedApps(tab.ownerUserId))
        }, kind = WontonButtonKind.SECONDARY)
        WontonButton(stringResource(R.string.common_close), { tabs.send(Action.ClosePage) },
            kind = WontonButtonKind.SECONDARY)
    }
}
