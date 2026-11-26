package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.StoreLoadState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 商店内容尚不可用：等待连接不是正在加载，也不代表已确定离线。 */
@Composable
internal fun StorePlaceholder(plugins: PluginsState, onPair: () -> Unit) {
    val reload = { plugins.refreshStore(force = true) }
    Column(Modifier.fillMaxSize()) {
        StoreRefreshRejection(plugins)
        when (val load = plugins.viewModel.storeLoad) {
            StoreLoadState.NotRequested -> StoreLoadMessage(
                stringResource(R.string.store_not_requested),
                onLoad = reload,
            )
            StoreLoadState.WaitingForConnection -> StoreLoadMessage(
                stringResource(R.string.store_waiting_for_connection),
            )
            StoreLoadState.NotPaired -> notPairedMessage(onPair)
            StoreLoadState.Loading -> Column(Modifier.fillMaxSize()) {
                Column(Modifier.weight(1f)) {
                    ListSkeleton(WontonTheme.size.storeIcon, slowHint = stringResource(R.string.store_pending))
                }
                WontonButton(
                    stringResource(R.string.store_reload),
                    reload,
                    Modifier.align(Alignment.CenterHorizontally).padding(bottom = WontonTheme.spacing.pageInset),
                    kind = WontonButtonKind.SECONDARY,
                )
            }
            is StoreLoadState.Failed ->
                if (load.problem.reason == PluginFailure.NOT_PAIRED) {
                    notPairedMessage(onPair)
                } else {
                    ProblemView(load.problem.problem, reload)
                }
            StoreLoadState.Ready -> Unit
        }
    }
}

/** 已有内容仍保留；商店加载失败和操作拒绝独立展示，不借用已安装列表的问题。 */
@Composable
internal fun StoreLoadProblems(plugins: PluginsState) {
    Column {
        when (val load = plugins.viewModel.storeLoad) {
            StoreLoadState.NotPaired -> Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
                ProblemRow(
                    Problem(
                        titleRes = R.string.problem_not_paired_title,
                        nextStepRes = R.string.problem_not_paired_next_step,
                    )
                ) { plugins.refreshStore(force = true) }
            }
            is StoreLoadState.Failed -> Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
                ProblemRow(load.problem.problem) { plugins.refreshStore(force = true) }
            }
            else -> Unit
        }
        StoreRefreshRejection(plugins)
    }
}

@Composable
private fun StoreRefreshRejection(plugins: PluginsState) {
    plugins.storeRefreshRejection?.let { rejection ->
        Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
            ProblemRow(rejection.problem) { plugins.refreshStore(force = true) }
        }
    }
}

@Composable
private fun StoreLoadMessage(
    text: String,
    detail: String? = null,
    onLoad: (() -> Unit)? = null,
    loadLabel: String = stringResource(R.string.store_load),
) {
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text,
            style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        detail?.let {
            Text(
                it,
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
        onLoad?.let {
            WontonButton(loadLabel, it, kind = WontonButtonKind.SECONDARY)
        }
    }
}

@Composable
private fun notPairedMessage(onPair: () -> Unit) {
    StoreLoadMessage(
        stringResource(R.string.problem_not_paired_title),
        detail = stringResource(R.string.problem_not_paired_next_step),
        onLoad = onPair,
        loadLabel = stringResource(R.string.store_go_pair),
    )
}
