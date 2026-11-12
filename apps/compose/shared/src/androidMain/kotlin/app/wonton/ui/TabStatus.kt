package app.wonton.ui

import android.icu.text.ListFormatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
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
    page: TabPage, tab: TabRef, tabs: PagesState, problem: Problem?, supported: Boolean,
    crossOriginBlocked: Boolean, onDetails: (String) -> Unit, place: PlacedTab? = null,
) {
    val state = page.state
    // 核心让它在系统浏览器里打开（ADR-0033 的“跨源隔离”）：App 里只留一行说明。
    val inBrowser = crossOriginBlocked && problem == null &&
        (state is TabState.Ready || state is TabState.Upgrading)
    if (inBrowser) {
        BrowserPageStatus(page, tabs.core, autoOpen = place !is PlacedTab.Panel)
        return
    }
    val failure = when {
        !supported -> Problem(stringResource(R.string.tab_webview_too_old))
        problem != null -> problem
        state is TabState.Stopped -> Problem(stringResource(R.string.tab_stopped, page.title))
        state is TabState.Failed -> Problem(stringResource(R.string.tab_problem, page.title), detail = state.reason)
        state is TabState.PageRemoved -> Problem(stringResource(R.string.tab_page_removed))
        state is TabState.Unknown -> Problem(stringResource(R.string.tab_status_unknown))
        state is TabState.LoadFailed -> Problem(
            loadFailureTitle(state.failure),
            // 详情是核心按 App 语言写的（错误码收在里面）；核心没给时退回裸错误码。
            detail = state.details ?: state.status?.let { "HTTP $it" },
        )
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
            TabStatusActions(state, tab, tabs, onDetails, place)
        }
    }
}

@Composable
private fun TabStatusActions(
    state: TabState, tab: TabRef, tabs: PagesState, onDetails: (String) -> Unit, place: PlacedTab?,
) {
    val pluginId = tab.localPluginId()
    if (state is TabState.PageRemoved && place is PlacedTab.Panel) {
        WontonButton(stringResource(R.string.panel_remove), {
            tabs.send(Action.RemovePanelPage(place.scope, tab))
        })
    } else if (state is TabState.PageRemoved) {
        WontonButton(stringResource(R.string.common_close), { tabs.send(Action.ClosePage) })
    } else if (state is TabState.Stopped && tab is TabRef.Plugin) {
        WontonButton(stringResource(R.string.tab_start), { tabs.send(Action.StartPlugin(tab.pluginId)) })
    } else {
        WontonButton(stringResource(R.string.common_retry), {
            tabs.send(Action.ReloadTab(place ?: PlacedTab.Page(tab)))
        })
        if (state is TabState.Failed && pluginId != null) {
            WontonButton(stringResource(R.string.tab_view_details), { onDetails(pluginId) })
        }
    }
}

@Composable
private fun loadFailureTitle(failure: TabLoadFailure): String = when (failure) {
    TabLoadFailure.TRANSPORT -> stringResource(R.string.problem_transport_unavailable_title)
    TabLoadFailure.PLUGIN_NOT_READY -> stringResource(R.string.tab_load_not_ready)
    TabLoadFailure.UPSTREAM -> stringResource(R.string.tab_load_upstream)
}

/** 已卸载的应用从我的应用里拿掉了：说一次。我的应用读写的问题不弹窗，在侧边栏的“我的应用”下面轻提示。 */
@Composable
fun TabNotices(tabs: PagesState) {
    val view = tabs.viewModel
    if (view.uninstalled.isNotEmpty()) {
        val names = ListFormatter.getInstance(LocalConfiguration.current.locales[0]).format(view.uninstalled)
        val dismiss = { tabs.send(Action.DismissUninstalledTabs) }
        WontonDialog(
            stringResource(R.string.tab_uninstalled_notice, names),
            onDismiss = dismiss,
            actions = listOf(WontonDialogAction(stringResource(R.string.design_ok), onClick = dismiss)),
        )
    }
}
