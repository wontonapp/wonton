package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 侧边栏“我的应用”下面的轻提示：账号数据读不了（断线）时列表照常用缓存显示，这里只说可能不是最新的并给重试；
 * 我的应用读写出错时同样只在这里说，原因收进默认收起的“详情”。不弹窗、不打断（盲测 #8）。
 */
@Composable
internal fun MyAppsSyncHint(pages: PagesState, interactive: Boolean) {
    val view = pages.viewModel
    val problem = view.problem
    if (!view.syncStale && problem == null) return
    val spacing = WontonTheme.spacing
    val colors = WontonTheme.colors
    Column(
        Modifier.padding(horizontal = spacing.s12),
        verticalArrangement = Arrangement.spacedBy(spacing.s4),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(if (problem != null) R.string.myapp_problem else R.string.myapp_sync_stale),
                Modifier.weight(1f),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
            Spacer(Modifier.widthIn(min = spacing.s8))
            if (view.syncStale) {
                TextButton(onClick = { pages.send(Action.Pages(PageAction.RetryMyAppSync)) }, enabled = interactive) {
                    Text(stringResource(R.string.common_retry), style = WontonTheme.typography.supporting,
                        color = colors.accentInk.toColor())
                }
            }
        }
        ProblemDetail(problem?.message ?: stringResource(R.string.myapp_sync_stale_detail))
    }
}
