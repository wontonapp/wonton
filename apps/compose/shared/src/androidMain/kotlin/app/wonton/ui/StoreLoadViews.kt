package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
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
    StorePlaceholder(
        plugins.viewModel.storeLoad,
        onReload = reload,
        onPair = onPair,
        rejection = plugins.storeRefreshRejection?.problem,
    )
}

/** [StorePlaceholder] 的纯渲染版本：状态和动作都由调用方给，Compose 测试直接渲染。 */
@Composable
fun StorePlaceholder(
    load: StoreLoadState,
    onReload: () -> Unit,
    onPair: () -> Unit,
    rejection: Problem? = null,
) {
    Column(Modifier.fillMaxSize()) {
        rejection?.let {
            Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
                ProblemRow(it) { onReload() }
            }
        }
        when (load) {
            StoreLoadState.NotRequested -> StoreLoadMessage(
                stringResource(R.string.store_not_requested),
                onLoad = onReload,
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
                    onReload,
                    Modifier.align(Alignment.CenterHorizontally).padding(bottom = WontonTheme.spacing.pageInset),
                    kind = WontonButtonKind.SECONDARY,
                )
            }
            is StoreLoadState.Failed ->
                if (load.problem.reason == PluginFailure.NOT_PAIRED) {
                    notPairedMessage(onPair)
                } else {
                    ProblemView(load.problem.problem, onReload)
                }
            StoreLoadState.Ready -> Unit
        }
    }
}

/** 已有内容仍保留；商店加载失败和操作拒绝独立展示，不借用已安装列表的问题。 */
@Composable
internal fun StoreLoadProblems(plugins: PluginsState) {
    StoreLoadProblems(
        plugins.viewModel.storeLoad,
        rejection = plugins.storeRefreshRejection?.problem,
        onReload = { plugins.refreshStore(force = true) },
    )
}

/** [StoreLoadProblems] 的纯渲染版本：还没配对时不补只会回到同一状态的重试。 */
@Composable
private fun StoreLoadProblems(
    load: StoreLoadState,
    rejection: Problem?,
    onReload: () -> Unit,
) {
    Column {
        when (load) {
            StoreLoadState.NotPaired -> Unit
            is StoreLoadState.Failed ->
                if (load.problem.reason != PluginFailure.NOT_PAIRED) {
                    Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
                        ProblemRow(load.problem.problem, onReload)
                    }
                }
            else -> Unit
        }
        rejection?.let {
            Column(Modifier.padding(WontonTheme.spacing.pageInset)) {
                ProblemRow(it) { onReload() }
            }
        }
    }
}

@Composable
private fun StoreLoadMessage(
    text: String,
    detail: String? = null,
    onLoad: (() -> Unit)? = null,
    loadLabel: String = stringResource(R.string.store_load),
    loadTag: String? = null,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
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
            val buttonModifier = loadTag?.let { tag -> Modifier.testTag(tag) } ?: Modifier
            WontonButton(loadLabel, it, buttonModifier, kind = WontonButtonKind.SECONDARY)
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
        loadTag = "store-go-pair",
        modifier = Modifier.testTag("store-not-paired"),
    )
}
