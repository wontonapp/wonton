package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 侧边栏“我的应用”下面的轻提示，标题和说明上下排：本机改过、还没写进账号数据时说“改动还没同步上”
 * （`syncStale`，核心连上时和之后定时自己再同步，同步上了就收掉，所以不放按钮）；我的应用读写出错时同样只在这里说，
 * 原因收进默认收起的“详情”。不弹窗、不打断（盲测 #8）。
 */
@Composable
internal fun MyAppsSyncHint(pages: PagesState) {
    val view = pages.viewModel
    val problem = view.problem
    if (!view.syncStale && problem == null) return
    val spacing = WontonTheme.spacing
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxWidth().padding(horizontal = spacing.s12, vertical = spacing.s8).testTag("myAppsSyncStale"),
        verticalArrangement = Arrangement.spacedBy(spacing.s4),
    ) {
        Text(
            stringResource(if (problem != null) R.string.myapp_problem else R.string.myapp_sync_stale),
            style = WontonTheme.typography.supporting,
            color = colors.textPrimary.toColor(),
        )
        if (problem != null) {
            ProblemDetail(problem.message)
        } else {
            Text(
                stringResource(R.string.myapp_sync_stale_detail),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}
